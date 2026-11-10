use quote::quote;
use std::{env, fs, path::PathBuf};
use syn::{ForeignItem, Item, ReturnType, Type, Visibility};

/// Keep the absent-provider surface in lockstep with the audited bridge.
/// This code generates Rust data and errors, never an FFI call or a solid.
pub(crate) fn generate() {
    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=build/no_occt.rs");
    let source = fs::read_to_string("src/lib.rs").expect("read kernel bridge");
    let syntax = syn::parse_file(&source).expect("parse kernel bridge");
    let items = syntax
        .items
        .into_iter()
        .find_map(|item| match item {
            Item::Mod(module) if module.ident == "ffi" => module.content.map(|(_, items)| items),
            _ => None,
        })
        .expect("kernel bridge module");
    let mut declarations = Vec::new();
    for item in items {
        match item {
            Item::Struct(mut data) => {
                for field in &mut data.fields {
                    field.vis = Visibility::Public(Default::default());
                }
                declarations.push(quote!(#data));
            }
            Item::ForeignMod(bridge) => {
                for item in bridge.items {
                    declarations.push(match item {
                        ForeignItem::Type(kind) => {
                            let name = kind.ident;
                            quote!(pub(crate) enum #name {})
                        }
                        ForeignItem::Fn(function) => {
                            let signature = function.sig;
                            let body = match &signature.output {
                                ReturnType::Type(_, output)
                                    if matches!(&**output, Type::Path(path)
                                        if path.path.segments.first().is_some_and(|s| s.ident == "Result")) =>
                                {
                                    quote!(Err(Unavailable))
                                }
                                ReturnType::Default
                                    if signature.ident == "route_native_output_to_stderr" =>
                                {
                                    quote!()
                                }
                                ReturnType::Type(_, output)
                                    if signature.ident.to_string().starts_with("shares_")
                                        && matches!(&**output, Type::Path(path) if path.path.is_ident("bool")) =>
                                {
                                    quote!(false)
                                }
                                _ => panic!("Define absent-provider behavior for {}", signature.ident),
                            };
                            quote!(pub(crate) #signature { #body })
                        }
                        ForeignItem::Macro(item) if item.mac.path.is_ident("include") => continue,
                        _ => panic!("Unexpected declaration in kernel bridge"),
                    });
                }
            }
            _ => panic!("Unexpected item in kernel bridge"),
        }
    }
    let output =
        PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output directory")).join("no_occt.rs");
    fs::write(output, quote!(#(#declarations)*).to_string()).expect("write absent kernel surface");
}
