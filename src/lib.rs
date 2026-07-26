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

fn should_skip_method(method: &syn::TraitItemFn) -> bool {
    // 0. 明示的なスキップ属性
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

// 参照のミュータビリティを判定するヘルパー
fn is_mut_method(method: &syn::TraitItemFn) -> bool {
    if let Some(syn::FnArg::Receiver(recv)) = method.sig.inputs.first() {
        recv.mutability.is_some()
    } else {
        false
    }
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

    // メソッドを &self 用と &mut self 用に分別する
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

    // 1. 不変参照用のタプルトレイト (TraitTuple) の構築
    let ref_tuple_trait_ident = format_ident!("{}Tuple", trait_ident);
    let mut ref_tuple_trait = input_trait.clone();
    ref_tuple_trait.ident = ref_tuple_trait_ident.clone();
    ref_tuple_trait
        .generics
        .params
        .insert(0, syn::parse_quote!(const N: usize));
    ref_tuple_trait.items = ref_trait_items;

    // 2. 可変参照用のタプルトレイト (TraitMutTuple) の構築
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

        // 実装メソッドを生成するクロージャ
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

        // [実装A] 不変参照タプルに TraitTuple(&self のみ) を実装
        impls.push(quote! {
            impl #ref_impl_generics #ref_tuple_trait_ident <#n, #(#generic_args),*> for #ref_tuple
            #impl_where
            {
                #(#ref_impl_methods)*
            }
        });

        // [実装B] 可変参照タプルにも TraitTuple(&self のみ) を実装
        impls.push(quote! {
            impl #ref_impl_generics #ref_tuple_trait_ident <#n, #(#generic_args),*> for #mut_tuple
            #impl_where
            {
                #(#ref_impl_methods)*
            }
        });

        // [実装C] 可変参照タプルに TraitMutTuple(&mut self のみ) を実装 (存在する場合)
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

    // 元のトレイトから `#[tuple_skip]` を消去
    for item in &mut input_trait.items {
        if let TraitItem::Fn(method) = item {
            method
                .attrs
                .retain(|attr| !attr.path().is_ident("skip_gtuple"));
        }
    }

    // トレイトと実装を結合して出力
    let mut expanded = quote! {
        #input_trait
        #ref_tuple_trait
        #mut_tuple_trait
    };
    /*
    if !mut_methods_orig.is_empty() {
        expanded.extend(quote! {
            #mut_tuple_trait
        });
    }*/

    expanded.extend(quote! {
        #(#impls)*
    });

    TokenStream::from(expanded)
}
