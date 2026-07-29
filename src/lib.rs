//! # gtuple_monomorphization
//! `gtuple_monomorphization` は、「同じトレイトの複数の異なる型」をタプルにしたときに、一括でメソッドを呼び出せるようにする属性マクロです。
//!
//! # 1. 基本的な使用例
//!
//! ```rust
//! use gtuple::gtuple;
//!
//! #[gtuple(1, 2)]
//! pub trait Alphabet {
//!     fn to_char(&self) -> char;
//! }
//!
//! struct A;
//! impl Alphabet for A {
//!     fn to_char(&self) -> char { 'A' }
//! }
//!
//! struct B;
//! impl Alphabet for B {
//!     fn to_char(&self) -> char { 'B' }
//! }
//!
//! let a = A;
//! let b = B;
//!
//! // 不変参照のタプルに対して一括適用し、配列で取得
//! assert_eq!((&a, &b).to_char(), ['A', 'B']);
//! ```
//!
//! # 2. マクロによってどのように展開されるか
//!
//! 例として、Alphabetトレイトに `#[gtuple]` を付与した場合、マクロ内部では元々の `Alphabet` トレイトに加えて
//! **`AlphabetTuple<const N: usize>`** および **`AlphabetMutTuple<const N: usize>`** が自動生成され、
//! タプル型 `(&T0, &T1)` 等に対して実装が生成されます。
//!
//! 手動で書いた場合の「マクロ展開後のコード」と同等なコード例は以下の通りです：
//!
//! ```rust
//! // 1. 元のトレイト
//! pub trait Alphabet {
//!     fn to_char(&self) -> char;
//! }
//!
//! // 2. マクロによって生成される &self 用のタプルトレイト
//! //    (戻り値が `char` から配列 `[char; N]` に変化します)
//! pub trait AlphabetTuple<const N: usize> {
//!     fn to_char(&self) -> [char; N];
//! }
//!
//! // 3. マクロによって生成される &mut self 用のタプルトレイト
//! pub trait AlphabetMutTuple<const N: usize> {
//!     fn to_char(&self) -> [char; N];
//! }
//!
//! // 4. マクロによって自動生成される実装 (要素数 N=2 の例) ---
//! impl<'__tuple_macro_lt, T0, T1> AlphabetTuple<2> for (&'__tuple_macro_lt T0, &'__tuple_macro_lt T1)
//! where
//!     T0: Alphabet,
//!     T1: Alphabet,
//! {
//!     fn to_char(&self) -> [char; 2] {
//!         [self.0.to_char(), self.1.to_char()]
//!     }
//! }
//!
//! // --- 構造体の実装 ---
//! struct A;
//! impl Alphabet for A {
//!     fn to_char(&self) -> char { 'A' }
//! }
//!
//! struct B;
//! impl Alphabet for B {
//!     fn to_char(&self) -> char { 'B' }
//! }
//!
//! let a = A;
//! let b = B;
//!
//! // タプル参照に対して AlphabetTuple::to_char が呼び出されます
//! assert_eq!((&a, &b).to_char(), ['A', 'B']);
//! ```

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream, Result};
use syn::{ItemTrait, LitInt, ReturnType, Token, TraitItem, parse_macro_input};

/// 戻り値がユニット型 `()` (または明示的な `Default`) であるか判定するヘルパー。
fn is_unit(output: &ReturnType) -> bool {
    match output {
        ReturnType::Default => true,
        ReturnType::Type(_, ty) => {
            matches!(&**ty, syn::Type::Tuple(tup) if tup.elems.is_empty())
        }
    }
}

/// 引数の型が「各要素へ分配不可能（Moveが必要）」かどうかを判定します。
fn is_not_distributable(ty: &syn::Type) -> bool {
    match ty {
        syn::Type::Reference(_) => false,
        syn::Type::Ptr(_) => false,
        syn::Type::Path(type_path) => {
            if let Some(ident) = type_path.path.get_ident() {
                let name = ident.to_string();
                let primitives = [
                    "usize", "u8", "u16", "u32", "u64", "u128", "isize", "i8", "i16", "i32", "i64",
                    "i128", "f32", "f64", "bool", "char",
                ];
                if primitives.contains(&name.as_str()) {
                    return false;
                }
            }
            true
        }
        _ => true,
    }
}

/// 戻り値の型が Sized でない、または固定長配列 `[T; N]` にパック不可能な型であるか判定します。
fn is_unsized_or_unmappable_type(ty: &syn::Type) -> bool {
    match ty {
        syn::Type::Slice(_) => true,
        syn::Type::TraitObject(_) => true,
        syn::Type::Path(type_path) => {
            if let Some(ident) = type_path.path.get_ident() {
                let name = ident.to_string();
                if name == "str" || name == "Self" {
                    return true;
                }
            }
            false
        }
        _ => false,
    }
}

/// 指定されたトレイトメソッドがタプル実装の対象からスキップされるべきかを判定します。
fn should_skip_method(method: &syn::TraitItemFn) -> bool {
    if method
        .attrs
        .iter()
        .any(|attr| attr.path().is_ident("skip_gtuple"))
    {
        return true;
    }

    let sig = &method.sig;
    let inputs = &sig.inputs;

    match inputs.first() {
        Some(syn::FnArg::Receiver(recv)) => {
            if recv.reference.is_none() {
                return true;
            }
        }
        _ => return true,
    }

    for arg in inputs.iter().skip(1) {
        if let syn::FnArg::Typed(pat_type) = arg {
            if is_not_distributable(&pat_type.ty) {
                return true;
            }
        }
    }

    if let ReturnType::Type(_, ty) = &sig.output {
        if is_unsized_or_unmappable_type(ty) {
            return true;
        }
    }

    false
}

/// メソッドのレシーバが可変参照 (`&mut self`) かどうか判定するヘルパー。
fn is_mut_method(method: &syn::TraitItemFn) -> bool {
    if let Some(syn::FnArg::Receiver(recv)) = method.sig.inputs.first() {
        recv.mutability.is_some()
    } else {
        false
    }
}

/// マクロに引き渡されるタプル要素数の範囲引数をパースする構造体。
struct RangeArgs {
    start: usize,
    end: usize,
}

impl Parse for RangeArgs {
    fn parse(input: ParseStream) -> Result<Self> {
        if input.is_empty() {
            return Ok(RangeArgs { start: 2, end: 12 });
        }
        let start_lit: LitInt = input.parse()?;
        let start = start_lit.base10_parse::<usize>()?;

        input.parse::<Token![,]>()?;

        let end_lit: LitInt = input.parse()?;
        let end = end_lit.base10_parse::<usize>()?;

        Ok(RangeArgs { start, end })
    }
}

/// トレイトに対して、参照のタプル (`(&T1, &T2, ...)` / `(&mut T1, &mut T2, ...)`) 向けの一括実行用トレイトと実装を自動生成します。
///
/// # 引数
/// * `start, end` (任意): 生成対象とするタプルの最小・最大要素数。指定しない場合のデフォルトは `1, 12` です。
///
/// # 生成されるトレイト
/// 対象のトレイト名を `Foo` とした場合、以下の2つのトレイトが生成されます：
/// * **`FooTuple<const N: usize>`**: `&self` メソッドを集約して実行するトレイト。
/// * **`FooMutTuple<const N: usize>`**: `&mut self` メソッドを集約して実行するトレイト。
///
/// # メソッド戻り値の変換ルール
/// * **ユニット型 (`()`)**: 各要素のメソッドを順番に実行します（戻り値なし）。
/// * **値を持つ型 (`T`)**: 各要素のメソッドの戻り値を要素数 `N` の固定長配列 `[T; N]` として返します。
///
/// # Examples
///
/// ```rust
/// use gtuple::gtuple;

/// #[gtuple(1, 2)]
/// pub trait Alphabet {
///     fn to_char(&self) -> char;
/// }
///
/// struct A;
/// impl Alphabet for A {
///     fn to_char(&self) -> char {
///         'A'
///     }
/// }
///
/// struct B;
/// impl Alphabet for B {
///     fn to_char(&self) -> char {
///         'B'
///     }
/// }
///
/// let a = A;
/// let b = B;
///
/// // 不変参照のタプルから各要素の to_char を一括実行し、配列として結果を取得
/// assert_eq!((&a, &b).to_char(), ['A', 'B']);
/// ```
#[proc_macro_attribute]
pub fn gtuple(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as RangeArgs);
    let min_n = args.start;
    let max_n = args.end;

    let mut input_trait = parse_macro_input!(item as ItemTrait);
    let trait_ident = &input_trait.ident;
    let trait_generics = &input_trait.generics;
    let trait_where = &input_trait.generics.where_clause;

    let generic_args: Vec<_> = trait_generics
        .params
        .iter()
        .map(|p| match p {
            syn::GenericParam::Type(t) => {
                let id = &t.ident;
                quote!(#id)
            }
            syn::GenericParam::Lifetime(l) => {
                let id = &l.lifetime;
                quote!(#id)
            }
            syn::GenericParam::Const(c) => {
                let id = &c.ident;
                quote!(#id)
            }
        })
        .collect();

    let mut ref_methods_orig = Vec::new();
    let mut mut_methods_orig = Vec::new();

    for item in &input_trait.items {
        if let TraitItem::Fn(method) = item {
            if should_skip_method(method) {
                continue;
            }
            if is_mut_method(method) {
                mut_methods_orig.push(method.clone());
            } else {
                ref_methods_orig.push(method.clone());
            }
        }
    }

    let transform_to_trait_item = |method: &syn::TraitItemFn| -> syn::TraitItem {
        let mut new_method = method.clone();
        if !is_unit(&new_method.sig.output) {
            let ReturnType::Type(_, ty) = &new_method.sig.output else {
                unreachable!()
            };
            new_method.sig.output = syn::parse_quote!(-> [#ty; N]);
        }
        new_method.default = None;
        new_method
            .attrs
            .retain(|attr| !attr.path().is_ident("skip_gtuple"));
        TraitItem::Fn(new_method)
    };

    let ref_trait_items: Vec<_> = ref_methods_orig
        .iter()
        .map(transform_to_trait_item)
        .collect();
    let mut_trait_items: Vec<_> = mut_methods_orig
        .iter()
        .map(transform_to_trait_item)
        .collect();

    let ref_tuple_trait_ident = format_ident!("{}Tuple", trait_ident);
    let mut ref_tuple_trait = input_trait.clone();
    ref_tuple_trait.ident = ref_tuple_trait_ident.clone();
    ref_tuple_trait
        .generics
        .params
        .insert(0, syn::parse_quote!(const N: usize));
    ref_tuple_trait.items = ref_trait_items;

    let mut_tuple_trait_ident = format_ident!("{}MutTuple", trait_ident);
    let mut mut_tuple_trait = input_trait.clone();
    mut_tuple_trait.ident = mut_tuple_trait_ident.clone();
    mut_tuple_trait
        .generics
        .params
        .insert(0, syn::parse_quote!(const N: usize));
    mut_tuple_trait.items = mut_trait_items;

    let mut impls = Vec::new();

    for n in min_n..=max_n {
        let tuple_idents: Vec<_> = (0..n).map(|i| format_ident!("T{}", i)).collect();
        let indices: Vec<_> = (0..n).map(syn::Index::from).collect();

        let mut impl_generics = trait_generics.clone();
        for t_ident in tuple_idents.iter().rev() {
            impl_generics.params.insert(0, syn::parse_quote!(#t_ident));
        }

        let mut ref_impl_generics = impl_generics.clone();
        ref_impl_generics
            .params
            .insert(0, syn::parse_quote!('__tuple_macro_lt));

        let mut impl_where = trait_where
            .clone()
            .unwrap_or_else(|| syn::parse_quote!(where));
        for t_ident in &tuple_idents {
            impl_where
                .predicates
                .push(syn::parse_quote!(#t_ident: #trait_ident <#(#generic_args),*>));
        }

        let generate_impl_method = |method: &syn::TraitItemFn| -> proc_macro2::TokenStream {
            let sig = &method.sig;
            let method_ident = &sig.ident;

            let mut arg_names = Vec::new();
            for arg in &sig.inputs {
                if let syn::FnArg::Typed(pat_type) = arg {
                    if let syn::Pat::Ident(pat_ident) = &*pat_type.pat {
                        let id = &pat_ident.ident;
                        arg_names.push(quote!(#id));
                    }
                }
            }

            let args_tokens = quote! { #(#arg_names),* };
            let mut new_sig = sig.clone();
            let is_ret_unit = is_unit(&sig.output);

            if !is_ret_unit {
                let ReturnType::Type(_, ty) = &sig.output else {
                    unreachable!()
                };
                new_sig.output = syn::parse_quote!(-> [#ty; #n]);
            }

            let body = if is_ret_unit {
                quote! { #( self.#indices.#method_ident( #args_tokens ) ; )* }
            } else {
                quote! { [ #( self.#indices.#method_ident( #args_tokens ) ),* ] }
            };

            new_sig.inputs = new_sig.inputs.into_iter().collect();

            quote! {
                #new_sig {
                    #body
                }
            }
        };

        let ref_impl_methods: Vec<_> = ref_methods_orig.iter().map(generate_impl_method).collect();
        let mut_impl_methods: Vec<_> = mut_methods_orig.iter().map(generate_impl_method).collect();

        let mut_tuple = quote!( (#(&'__tuple_macro_lt mut #tuple_idents,)*) );
        let ref_tuple = quote!( (#(&'__tuple_macro_lt #tuple_idents,)*) );

        impls.push(quote! {
            impl #ref_impl_generics #ref_tuple_trait_ident <#n, #(#generic_args),*> for #ref_tuple
            #impl_where
            {
                #(#ref_impl_methods)*
            }
        });

        impls.push(quote! {
            impl #ref_impl_generics #ref_tuple_trait_ident <#n, #(#generic_args),*> for #mut_tuple
            #impl_where
            {
                #(#ref_impl_methods)*
            }
        });

        if !mut_methods_orig.is_empty() {
            impls.push(quote! {
                impl #ref_impl_generics #mut_tuple_trait_ident <#n, #(#generic_args),*> for #mut_tuple
                #impl_where
                {
                    #(#mut_impl_methods)*
                }
            });
        }
    }

    for item in &mut input_trait.items {
        if let TraitItem::Fn(method) = item {
            method
                .attrs
                .retain(|attr| !attr.path().is_ident("skip_gtuple"));
        }
    }

    let mut expanded = quote! {
        #input_trait
        #ref_tuple_trait
        #mut_tuple_trait
    };

    expanded.extend(quote! {
        #(#impls)*
    });

    TokenStream::from(expanded)
}
