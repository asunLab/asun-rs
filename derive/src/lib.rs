//! Derive macros for the asun serialization format.
//!
//! Provides `#[derive(AsunEncode)]` and `#[derive(AsunDecode)]`, which generate
//! implementations of all four runtime traits (`AsunEncode`, `AsunDecode`,
//! `AsunEncodeBinary`, `AsunDecodeBinary`) from the `asun` crate. The generated
//! code is direct and sequential — it drives the `Encoder`/`Decoder` primitives
//! straight, with no visitor indirection.
//!
//! # Field / variant attributes
//!
//! Attributes are written as `#[asun(...)]` and align with serde where the
//! format allows:
//!
//! - `rename = "name"` — rename the field/variant on the wire (schema field
//!   name or enum variant name).
//! - `skip` — never serialize, never deserialize; decodes to the default.
//! - `skip_serializing` — do not write; still read from the wire when present.
//! - `skip_deserializing` — do not read; always decode to the default.
//! - `skip_serializing_if = "path"` — text-only conditional skip; the predicate
//!   `fn(&T) -> bool` decides per value. Ignored by the binary format.
//! - `default = "path"` — value source (`fn() -> T`) for a field skipped on
//!   decode; falls back to `Default::default()` when absent.
//!
//! ## Semantics per format
//!
//! Text is self-describing, so a field can be skipped on one side only. Binary
//! has no schema and reads fields in declaration order, so **any skip is forced
//! symmetric** (skipped on one side ⇒ skipped on both), or field alignment
//! would break.
//!
//! | attribute | text encode | text decode | binary encode | binary decode |
//! |---|---|---|---|---|
//! | `skip` | omit | default | omit | default |
//! | `skip_serializing` | omit | read | omit (symmetric) | default |
//! | `skip_deserializing` | write | default | omit (symmetric) | default |
//! | `skip_serializing_if` | conditional omit | read | write (attr ignored) | read |
//!
//! Note: a custom `default = "path"` takes effect only for fields skipped on
//! *decode* (`skip` / `skip_deserializing`). A plain `skip_serializing` field
//! that happens to be absent from the text wire falls back to
//! `Default::default()`, because the runtime fills missing text fields before
//! the derive is consulted.

use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, Ident, LitStr, parse_macro_input};

/// Parsed `#[asun(...)]` attributes for a single field (or variant).
///
/// The three serde-aligned skip flags are normalized down to two booleans plus
/// an optional condition:
///   - `skip`                    ⇒ skip_ser = true, skip_de = true
///   - `skip_serializing`        ⇒ skip_ser = true
///   - `skip_deserializing`      ⇒ skip_de  = true
///   - `skip_serializing_if="p"` ⇒ text-only conditional skip (ignored in binary)
#[derive(Default)]
struct FieldAttrs {
    rename: Option<String>,
    /// Field is not written on serialize (text: omit; binary: omit + symmetric skip on decode).
    skip_ser: bool,
    /// Field is not read on deserialize (value comes from `default`).
    skip_de: bool,
    /// `skip_serializing_if = "path"` — text-only runtime predicate. Binary ignores it.
    skip_ser_if: Option<syn::Path>,
    /// `default = "path"` — source of the value when the field is skipped on decode.
    default: Option<syn::Path>,
}

impl FieldAttrs {
    /// The value expression for a skipped-on-decode field: the `default = "path"`
    /// function call if present, else `Default::default()`.
    fn default_expr(&self, ty: &syn::Type) -> proc_macro2::TokenStream {
        match &self.default {
            Some(p) => quote! { #p() },
            None => quote! { <#ty as ::core::default::Default>::default() },
        }
    }
}

/// Parse `#[asun(...)]` attributes into a `FieldAttrs`. Unknown keys are a
/// compile error (previously silently ignored, which let bad attributes no-op).
fn parse_field_attrs(attrs: &[syn::Attribute]) -> syn::Result<FieldAttrs> {
    let mut fa = FieldAttrs::default();
    for attr in attrs {
        if !attr.path().is_ident("asun") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename") {
                let s: LitStr = meta.value()?.parse()?;
                fa.rename = Some(s.value());
            } else if meta.path.is_ident("skip") {
                fa.skip_ser = true;
                fa.skip_de = true;
            } else if meta.path.is_ident("skip_serializing") {
                fa.skip_ser = true;
            } else if meta.path.is_ident("skip_deserializing") {
                fa.skip_de = true;
            } else if meta.path.is_ident("skip_serializing_if") {
                let s: LitStr = meta.value()?.parse()?;
                fa.skip_ser_if = Some(s.parse()?);
            } else if meta.path.is_ident("default") {
                let s: LitStr = meta.value()?.parse()?;
                fa.default = Some(s.parse()?);
            } else {
                return Err(meta.error("unknown asun attribute key"));
            }
            Ok(())
        })?;
    }
    Ok(fa)
}

/// The wire name for a field/variant: its rename override or its Rust ident.
fn wire_name(ident: &Ident, attrs: &[syn::Attribute]) -> String {
    parse_field_attrs(attrs)
        .ok()
        .and_then(|fa| fa.rename)
        .unwrap_or_else(|| ident.to_string())
}

#[proc_macro_derive(AsunEncode, attributes(asun))]
pub fn derive_asun_encode(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let (text_body, bin_body) = match &input.data {
        Data::Struct(data) => encode_struct_bodies(&data.fields),
        Data::Enum(data) => encode_enum_bodies(name, data),
        Data::Union(_) => {
            return syn::Error::new_spanned(name, "AsunEncode cannot be derived for unions")
                .to_compile_error()
                .into();
        }
    };

    let expanded = quote! {
        impl #impl_generics ::asun::AsunEncode for #name #ty_generics #where_clause {
            #[inline]
            fn encode(&self, __enc: &mut ::asun::encode::Encoder) -> ::asun::Result<()> {
                #text_body
            }
        }

        impl #impl_generics ::asun::AsunEncodeBinary for #name #ty_generics #where_clause {
            #[inline]
            fn encode_binary(&self, __enc: &mut ::asun::binary::BinaryEncoder) -> ::asun::Result<()> {
                #bin_body
            }
        }
    };
    expanded.into()
}

#[proc_macro_derive(AsunDecode, attributes(asun))]
pub fn derive_asun_decode(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

    // Decode needs a lifetime naming the input buffer. When the type already
    // has exactly one lifetime, reuse it: that is what makes `&'a str` fields
    // borrow straight out of the input instead of allocating. Always inserting
    // a fresh `'de` (as this used to) made every borrowing struct fail to
    // compile, so zero-copy decode was unreachable through the derive.
    let existing: Vec<syn::Lifetime> = input
        .generics
        .lifetimes()
        .map(|l| l.lifetime.clone())
        .collect();

    let (de_lifetime, de_generics) = if existing.len() == 1 {
        (existing[0].clone(), input.generics.clone())
    } else {
        let mut param: syn::LifetimeParam = syn::parse_quote!('de);
        // The input buffer must outlive every borrow the type holds.
        for lt in &existing {
            param.bounds.push(lt.clone());
        }
        let mut generics = input.generics.clone();
        generics
            .params
            .insert(0, syn::GenericParam::Lifetime(param));
        (syn::parse_quote!('de), generics)
    };

    let (impl_generics, _, _) = de_generics.split_for_impl();
    let (_, ty_generics, where_clause) = input.generics.split_for_impl();

    let (text_body, bin_body) = match &input.data {
        Data::Struct(data) => decode_struct_bodies(name, &data.fields, &de_lifetime),
        Data::Enum(data) => decode_enum_bodies(name, data, &de_lifetime),
        Data::Union(_) => {
            return syn::Error::new_spanned(name, "AsunDecode cannot be derived for unions")
                .to_compile_error()
                .into();
        }
    };

    let expanded = quote! {
        impl #impl_generics ::asun::AsunDecode<#de_lifetime> for #name #ty_generics #where_clause {
            #[inline]
            fn decode(__dec: &mut ::asun::decode::Decoder<#de_lifetime>) -> ::asun::Result<Self> {
                #text_body
            }
        }

        impl #impl_generics ::asun::AsunDecodeBinary<#de_lifetime> for #name #ty_generics #where_clause {
            #[inline]
            fn decode_binary(__dec: &mut ::asun::binary::BinaryDecoder<#de_lifetime>) -> ::asun::Result<Self> {
                #bin_body
            }
        }
    };
    expanded.into()
}

// ---------------------------------------------------------------------------
// Struct encode
// ---------------------------------------------------------------------------

fn encode_struct_bodies(fields: &Fields) -> (proc_macro2::TokenStream, proc_macro2::TokenStream) {
    match fields {
        Fields::Named(named) => {
            let mut text_fields = Vec::new();
            let mut bin_fields = Vec::new();
            for f in &named.named {
                let ident = f.ident.as_ref().unwrap();
                let fa = match parse_field_attrs(&f.attrs) {
                    Ok(fa) => fa,
                    Err(e) => {
                        let err = e.to_compile_error();
                        return (err.clone(), err);
                    }
                };
                let wname = fa.rename.clone().unwrap_or_else(|| ident.to_string());

                // Text: skip_ser omits the field entirely; skip_serializing_if
                // guards the field() call with a runtime predicate.
                if !fa.skip_ser {
                    if let Some(cond) = &fa.skip_ser_if {
                        text_fields.push(quote! {
                            if !#cond(&self.#ident) {
                                __st.field(__enc, #wname, &self.#ident)?;
                            }
                        });
                    } else {
                        text_fields.push(quote! {
                            __st.field(__enc, #wname, &self.#ident)?;
                        });
                    }
                }

                // Binary: no schema, so skip MUST be symmetric encode/decode or
                // field alignment breaks. A field that is skipped on either side
                // (skip_ser from skip/skip_serializing, OR skip_de from
                // skip/skip_deserializing) is omitted here AND on the decode side.
                // skip_serializing_if is ignored (binary always writes the field).
                if !fa.skip_ser && !fa.skip_de {
                    bin_fields.push(quote! {
                        ::asun::AsunEncodeBinary::encode_binary(&self.#ident, __enc)?;
                    });
                }
            }
            let field_count = named.named.len();
            let text = quote! {
                let mut __st = __enc.begin_struct(#field_count)?;
                #(#text_fields)*
                __st.end(__enc)
            };
            let bin = quote! {
                #(#bin_fields)*
                Ok(())
            };
            (text, bin)
        }
        Fields::Unnamed(unnamed) => {
            // Tuple struct → encoded as a tuple `(a,b,...)`.
            let mut text_fields = Vec::new();
            let mut bin_fields = Vec::new();
            for (i, _f) in unnamed.unnamed.iter().enumerate() {
                let idx = syn::Index::from(i);
                text_fields.push(quote! {
                    __enc.tuple_element(&self.#idx)?;
                });
                bin_fields.push(quote! {
                    ::asun::AsunEncodeBinary::encode_binary(&self.#idx, __enc)?;
                });
            }
            let text = quote! {
                __enc.begin_tuple()?;
                #(#text_fields)*
                __enc.end_tuple()
            };
            let bin = quote! {
                #(#bin_fields)*
                Ok(())
            };
            (text, bin)
        }
        Fields::Unit => {
            // Unit struct → serialized as unit `()`.
            let text = quote! { __enc.encode_unit() };
            let bin = quote! { Ok(()) };
            (text, bin)
        }
    }
}

// ---------------------------------------------------------------------------
// Struct decode
// ---------------------------------------------------------------------------

fn decode_struct_bodies(
    name: &Ident,
    fields: &Fields,
    de: &syn::Lifetime,
) -> (proc_macro2::TokenStream, proc_macro2::TokenStream) {
    match fields {
        Fields::Named(named) => {
            let idents: Vec<&Ident> = named
                .named
                .iter()
                .map(|f| f.ident.as_ref().unwrap())
                .collect();
            let types: Vec<&syn::Type> = named.named.iter().map(|f| &f.ty).collect();
            let attrs: Vec<FieldAttrs> = {
                let mut v = Vec::with_capacity(named.named.len());
                for f in &named.named {
                    match parse_field_attrs(&f.attrs) {
                        Ok(fa) => v.push(fa),
                        Err(e) => {
                            let err = e.to_compile_error();
                            return (err.clone(), err);
                        }
                    }
                }
                v
            };

            // Wire schema (target field list) only contains fields that are read
            // on decode — skip_de fields are absent so the Exact-mode positional
            // count and ByName missing-default logic both stay correct.
            let wnames: Vec<String> = attrs
                .iter()
                .zip(idents.iter())
                .filter(|(fa, _)| !fa.skip_de)
                .map(|(fa, id)| fa.rename.clone().unwrap_or_else(|| id.to_string()))
                .collect();

            // Positional (Exact) reads. skip_de fields consume no input — they
            // take their default and do NOT advance the positional index.
            let pos_reads =
                idents
                    .iter()
                    .zip(types.iter())
                    .zip(attrs.iter())
                    .map(|((id, ty), fa)| {
                        if fa.skip_de {
                            let dflt = fa.default_expr(ty);
                            quote! { let #id: #ty = #dflt; }
                        } else {
                            quote! {
                                let #id: #ty = __dec.struct_field_positional::<#ty>(__i)?;
                                __i += 1;
                            }
                        }
                    });

            // By-name (WithDefaults) matching: init Option slots (only for
            // read fields), match names, finalize.
            let opt_inits = idents
                .iter()
                .zip(attrs.iter())
                .filter(|(_, fa)| !fa.skip_de)
                .map(|(id, _)| quote! { let mut #id = ::core::option::Option::None; });
            let match_arms = idents
                .iter()
                .zip(types.iter())
                .zip(attrs.iter())
                .filter(|((_, _), fa)| !fa.skip_de)
                .map(|((id, ty), fa)| {
                    let wn = fa.rename.clone().unwrap_or_else(|| id.to_string());
                    quote! {
                        #wn => { #id = ::core::option::Option::Some(__dec.struct_field_value::<#ty>()?); }
                    }
                });
            let finalize = idents.iter().zip(types.iter()).zip(attrs.iter()).map(
                |((id, ty), fa)| {
                    if fa.skip_de {
                        let dflt = fa.default_expr(ty);
                        quote! { let #id: #ty = #dflt; }
                    } else {
                        quote! {
                            let #id: #ty = match #id {
                                ::core::option::Option::Some(v) => v,
                                ::core::option::Option::None => __dec.struct_field_default::<#ty>()?,
                            };
                        }
                    }
                },
            );

            let static_fields = quote! {
                &[#(#wnames),*]
            };

            let text = quote! {
                const __FIELDS: &'static [&'static str] = #static_fields;
                let __mode = __dec.begin_struct_decode(__FIELDS)?;
                let __result = match __mode {
                    ::asun::decode::StructDecodeMode::Exact => {
                        let mut __i = 0usize;
                        #(#pos_reads)*
                        let _ = __i;
                        #name { #(#idents),* }
                    }
                    ::asun::decode::StructDecodeMode::ByName => {
                        #(#opt_inits)*
                        while let ::core::option::Option::Some(__key) = __dec.next_struct_key()? {
                            match __key {
                                #(#match_arms)*
                                _ => { __dec.skip_struct_value()?; }
                            }
                        }
                        #(#finalize)*
                        #name { #(#idents),* }
                    }
                };
                __dec.end_struct_decode()?;
                Ok(__result)
            };

            // Binary: fields in declaration order, no schema. Skip must be
            // symmetric with encode — a field the encoder omits (skip_ser, which
            // includes skip) must NOT be read here, or field alignment breaks.
            // So the "don't read" condition is skip_ser || skip_de.
            let bin_reads = idents.iter().zip(types.iter()).zip(attrs.iter()).map(
                |((id, ty), fa)| {
                    if fa.skip_ser || fa.skip_de {
                        let dflt = fa.default_expr(ty);
                        quote! { let #id: #ty = #dflt; }
                    } else {
                        quote! { let #id: #ty = <#ty as ::asun::AsunDecodeBinary<#de>>::decode_binary(__dec)?; }
                    }
                },
            );
            let bin = quote! {
                #(#bin_reads)*
                Ok(#name { #(#idents),* })
            };
            (text, bin)
        }
        Fields::Unnamed(unnamed) => {
            let count = unnamed.unnamed.len();
            let types: Vec<&syn::Type> = unnamed.unnamed.iter().map(|f| &f.ty).collect();
            let text_reads = types.iter().map(|ty| {
                quote! { { let __v = __dec.tuple_element::<#ty>()?; __v } }
            });
            let text = quote! {
                __dec.begin_tuple()?;
                let __result = #name ( #(#text_reads),* );
                __dec.end_tuple(#count)?;
                Ok(__result)
            };
            let bin_reads = types.iter().map(|ty| {
                quote! { <#ty as ::asun::AsunDecodeBinary<#de>>::decode_binary(__dec)? }
            });
            let bin = quote! {
                Ok(#name ( #(#bin_reads),* ))
            };
            (text, bin)
        }
        Fields::Unit => {
            let text = quote! {
                __dec.decode_unit()?;
                Ok(#name)
            };
            let bin = quote! { Ok(#name) };
            (text, bin)
        }
    }
}

// ---------------------------------------------------------------------------
// Enum encode
// ---------------------------------------------------------------------------

fn encode_enum_bodies(
    name: &Ident,
    data: &syn::DataEnum,
) -> (proc_macro2::TokenStream, proc_macro2::TokenStream) {
    let mut text_arms = Vec::new();
    let mut bin_arms = Vec::new();

    for (variant_index, variant) in data.variants.iter().enumerate() {
        let vident = &variant.ident;
        let vname = wire_name(vident, &variant.attrs);
        let vidx = variant_index as u32;

        match &variant.fields {
            Fields::Unit => {
                text_arms.push(quote! {
                    #name::#vident => __enc.encode_unit_variant(#vname),
                });
                bin_arms.push(quote! {
                    #name::#vident => { __enc.write_variant_index(#vidx)?; Ok(()) }
                });
            }
            Fields::Unnamed(unnamed) if unnamed.unnamed.len() == 1 => {
                text_arms.push(quote! {
                    #name::#vident(__v0) => __enc.encode_newtype_variant(#vname, __v0),
                });
                bin_arms.push(quote! {
                    #name::#vident(__v0) => {
                        __enc.write_variant_index(#vidx)?;
                        ::asun::AsunEncodeBinary::encode_binary(__v0, __enc)
                    }
                });
            }
            Fields::Unnamed(unnamed) => {
                let bindings: Vec<Ident> = (0..unnamed.unnamed.len())
                    .map(|i| Ident::new(&format!("__v{i}"), vident.span()))
                    .collect();
                let text_elems = bindings.iter().map(|b| {
                    quote! { __tv.element(__enc, #b)?; }
                });
                let bin_elems = bindings.iter().map(|b| {
                    quote! { ::asun::AsunEncodeBinary::encode_binary(#b, __enc)?; }
                });
                text_arms.push(quote! {
                    #name::#vident( #(#bindings),* ) => {
                        let mut __tv = __enc.begin_tuple_variant(#vname)?;
                        #(#text_elems)*
                        __tv.end(__enc)
                    }
                });
                bin_arms.push(quote! {
                    #name::#vident( #(#bindings),* ) => {
                        __enc.write_variant_index(#vidx)?;
                        #(#bin_elems)*
                        Ok(())
                    }
                });
            }
            Fields::Named(named) => {
                let fidents: Vec<&Ident> = named
                    .named
                    .iter()
                    .map(|f| f.ident.as_ref().unwrap())
                    .collect();
                let text_elems = fidents.iter().map(|id| {
                    quote! { __sv.element(__enc, #id)?; }
                });
                let bin_elems = fidents.iter().map(|id| {
                    quote! { ::asun::AsunEncodeBinary::encode_binary(#id, __enc)?; }
                });
                text_arms.push(quote! {
                    #name::#vident { #(#fidents),* } => {
                        let mut __sv = __enc.begin_struct_variant(#vname)?;
                        #(#text_elems)*
                        __sv.end(__enc)
                    }
                });
                bin_arms.push(quote! {
                    #name::#vident { #(#fidents),* } => {
                        __enc.write_variant_index(#vidx)?;
                        #(#bin_elems)*
                        Ok(())
                    }
                });
            }
        }
    }

    let text = quote! {
        match self {
            #(#text_arms)*
        }
    };
    let bin = quote! {
        match self {
            #(#bin_arms)*
        }
    };
    (text, bin)
}

// ---------------------------------------------------------------------------
// Enum decode
// ---------------------------------------------------------------------------

fn decode_enum_bodies(
    name: &Ident,
    data: &syn::DataEnum,
    de: &syn::Lifetime,
) -> (proc_macro2::TokenStream, proc_macro2::TokenStream) {
    // Text: read variant name, dispatch.
    let mut text_arms = Vec::new();
    // Binary: read variant index, dispatch.
    let mut bin_arms = Vec::new();

    for (variant_index, variant) in data.variants.iter().enumerate() {
        let vident = &variant.ident;
        let vname = wire_name(vident, &variant.attrs);
        let vidx = variant_index as u32;

        match &variant.fields {
            Fields::Unit => {
                text_arms.push(quote! {
                    #vname => { __dec.finish_unit_variant()?; #name::#vident }
                });
                bin_arms.push(quote! {
                    #vidx => #name::#vident,
                });
            }
            Fields::Unnamed(unnamed) if unnamed.unnamed.len() == 1 => {
                let ty = &unnamed.unnamed[0].ty;
                text_arms.push(quote! {
                    #vname => {
                        let __v = __dec.newtype_variant_value::<#ty>()?;
                        #name::#vident(__v)
                    }
                });
                bin_arms.push(quote! {
                    #vidx => #name::#vident(<#ty as ::asun::AsunDecodeBinary<#de>>::decode_binary(__dec)?),
                });
            }
            Fields::Unnamed(unnamed) => {
                let count = unnamed.unnamed.len();
                let types: Vec<&syn::Type> = unnamed.unnamed.iter().map(|f| &f.ty).collect();
                let text_reads = types.iter().map(|ty| {
                    quote! { { let __e = __dec.tuple_element::<#ty>()?; __e } }
                });
                let bin_reads = types.iter().map(|ty| {
                    quote! { <#ty as ::asun::AsunDecodeBinary<#de>>::decode_binary(__dec)? }
                });
                text_arms.push(quote! {
                    #vname => {
                        __dec.begin_tuple_variant_body()?;
                        let __r = #name::#vident( #(#text_reads),* );
                        __dec.end_tuple(#count)?;
                        __r
                    }
                });
                bin_arms.push(quote! {
                    #vidx => #name::#vident( #(#bin_reads),* ),
                });
            }
            Fields::Named(named) => {
                let fidents: Vec<&Ident> = named
                    .named
                    .iter()
                    .map(|f| f.ident.as_ref().unwrap())
                    .collect();
                let ftypes: Vec<&syn::Type> = named.named.iter().map(|f| &f.ty).collect();
                let text_reads = fidents.iter().zip(ftypes.iter()).map(|(id, ty)| {
                    quote! { let #id: #ty = { let __e = __dec.tuple_element::<#ty>()?; __e }; }
                });
                let count = fidents.len();
                let bin_reads = fidents.iter().zip(ftypes.iter()).map(|(id, ty)| {
                    quote! { let #id: #ty = <#ty as ::asun::AsunDecodeBinary<#de>>::decode_binary(__dec)?; }
                });
                text_arms.push(quote! {
                    #vname => {
                        __dec.begin_tuple_variant_body()?;
                        #(#text_reads)*
                        __dec.end_tuple(#count)?;
                        #name::#vident { #(#fidents),* }
                    }
                });
                bin_arms.push(quote! {
                    #vidx => { #(#bin_reads)* #name::#vident { #(#fidents),* } },
                });
            }
        }
    }

    let text = quote! {
        let __variant = __dec.begin_enum()?;
        let __result = match __variant.as_str() {
            #(#text_arms)*
            __other => {
                return ::core::result::Result::Err(::asun::Error::Message(
                    ::std::format!("unknown variant `{}`", __other).into()
                ));
            }
        };
        __dec.end_enum()?;
        Ok(__result)
    };

    let bin = quote! {
        let __idx = __dec.read_variant_index()?;
        let __result = match __idx {
            #(#bin_arms)*
            __other => {
                return ::core::result::Result::Err(::asun::Error::Message(
                    ::std::format!("invalid variant index {}", __other).into()
                ));
            }
        };
        Ok(__result)
    };
    (text, bin)
}
