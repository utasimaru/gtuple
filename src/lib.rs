use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream, Result};
use syn::{ItemTrait, LitInt, ReturnType, Token, TraitItem, parse_macro_input};

fn is_unit(output: &ReturnType) -> bool {
    match output {
        ReturnType::Default => true,
        ReturnType::Type(_, ty) => {
            matches!(&**ty, syn::Type::Tuple(tup) if tup.elems.is_empty())
        }
    }
}

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

fn is_unsized_or_unmappable_type(ty: &syn::Type) -> bool {
    match ty {
        syn::Type::Slice(_) => true,
        syn::Type::TraitObject(_) => true,
        syn::Type::Path(type_path) => {
            if let Some(ident) = type_path.path.get_ident() {
                let name = ident.to_string();
                if name == "str" {
                    return true;
                }
                if name == "Self" {
                    return true;
                }
            }
            false
        }
        _ => false,
    }
}

// ★ 修正: 引数を TraitItemFn 全体に変更し、属性もチェックする
fn should_skip_method(method: &syn::TraitItemFn) -> bool {
    // 0. 明示的なスキップ属性 `#[skip_gtuple]` がある場合は除外
    if method
        .attrs
        .iter()
        .any(|attr| attr.path().is_ident("skip_gtuple"))
    {
        return true;
    }

    let sig = &method.sig;
    let inputs = &sig.inputs;

    // 1. スタティックメソッド、または self の所有権を奪う場合を除外
    match inputs.first() {
        Some(syn::FnArg::Receiver(recv)) => {
            if recv.reference.is_none() {
                return true;
            }
        }
        _ => return true,
    }

    // 2. 引数に move するものがないかチェック
    for arg in inputs.iter().skip(1) {
        if let syn::FnArg::Typed(pat_type) = arg {
            if is_not_distributable(&pat_type.ty) {
                return true;
            }
        }
    }

    // 3. 戻り値が Sized でない、または配列化不可能な型かチェック
    if let ReturnType::Type(_, ty) = &sig.output {
        if is_unsized_or_unmappable_type(ty) {
            return true;
        }
    }

    false
}

struct RangeArgs {
    start: usize,
    end: usize,
}

impl Parse for RangeArgs {
    fn parse(input: ParseStream) -> Result<Self> {
        if input.is_empty() {
            return Ok(RangeArgs { start: 1, end: 12 });
        }
        let start_lit: LitInt = input.parse()?;
        let start = start_lit.base10_parse::<usize>()?;

        input.parse::<Token![,]>()?;

        let end_lit: LitInt = input.parse()?;
        let end = end_lit.base10_parse::<usize>()?;

        Ok(RangeArgs { start, end })
    }
}

#[proc_macro_attribute]
pub fn gtuple(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as RangeArgs);
    let min_n = args.start;
    let max_n = args.end;

    // ★ 修正: ミュータブルにする (後で元のトレイトから属性を消すため)
    let mut input_trait = parse_macro_input!(item as ItemTrait);
    let trait_ident = &input_trait.ident;
    let trait_generics = &input_trait.generics;
    let trait_where = &input_trait.generics.where_clause;

    let tuple_trait_ident = format_ident!("{}Tuple", trait_ident);

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

    let mut tuple_trait = input_trait.clone();
    tuple_trait.ident = tuple_trait_ident.clone();
    tuple_trait
        .generics
        .params
        .insert(0, syn::parse_quote!(const N: usize));

    // メソッドのフィルタリング (タプルトレイト側)
    tuple_trait.items.retain_mut(|item| {
        if let TraitItem::Fn(method) = item {
            // ★ TraitItemFnをそのまま渡す
            if should_skip_method(method) {
                return false;
            }

            if !is_unit(&method.sig.output) {
                let ReturnType::Type(_, ty) = &method.sig.output else {
                    unreachable!()
                };
                method.sig.output = syn::parse_quote!(-> [#ty; N]);
            }
            method.default = None;

            // タプルトレイト側にコピーされたメソッドからは、念のため `skip_gtuple` 属性を消去しておく
            method
                .attrs
                .retain(|attr| !attr.path().is_ident("skip_gtuple"));

            true
        } else {
            true
        }
    });

    let mut impls = Vec::new();

    for n in min_n..=max_n {
        let tuple_idents: Vec<_> = (0..n).map(|i| format_ident!("T{}", i)).collect();
        let indices: Vec<_> = (0..n).map(syn::Index::from).collect();

        let mut impl_generics = trait_generics.clone();
        for t_ident in tuple_idents.iter().rev() {
            impl_generics.params.insert(0, syn::parse_quote!(#t_ident));
        }

        let mut impl_where = trait_where
            .clone()
            .unwrap_or_else(|| syn::parse_quote!(where));
        for t_ident in &tuple_idents {
            impl_where
                .predicates
                .push(syn::parse_quote!(#t_ident: #trait_ident <#(#generic_args),*>));
        }

        let mut methods = Vec::new();
        for item in &input_trait.items {
            if let TraitItem::Fn(method) = item {
                // ★ ここでも TraitItemFn をそのまま渡す
                if should_skip_method(method) {
                    continue;
                }

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

                // ★ 実装メソッドからも `skip_gtuple` 属性を取り除く
                new_sig.inputs = new_sig.inputs.into_iter().collect(); // (形式的な変換)

                methods.push(quote! {
                    #new_sig {
                        #body
                    }
                });
            }
        }

        let tuple_type = quote!( (#(#tuple_idents,)*) );

        impls.push(quote! {
            impl #impl_generics #tuple_trait_ident <#n, #(#generic_args),*> for #tuple_type
            #impl_where
            {
                #(#methods)*
            }
        });
    }

    // ★ 仕上げ: 元のトレイトから `#[skip_gtuple]` 属性を取り除く
    // そのまま出力するとコンパイラが「そんな属性知らないよ」とエラーにするため
    for item in &mut input_trait.items {
        if let TraitItem::Fn(method) = item {
            method
                .attrs
                .retain(|attr| !attr.path().is_ident("skip_gtuple"));
        }
    }

    let expanded = quote! {
        #input_trait
        #tuple_trait
        #(#impls)*
    };

    TokenStream::from(expanded)
}
