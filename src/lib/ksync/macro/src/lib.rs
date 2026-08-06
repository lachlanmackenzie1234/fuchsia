// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::spanned::Spanned;
use syn::{Fields, Ident, ItemStruct, Type, TypePath, parse_macro_input};

struct MutexField {
    ident: Ident,
    class_ident: Ident,
    mutex_type: proc_macro2::TokenStream,
    custom_class: Option<Ident>,
    is_phantom: bool,
}

struct GuardedField {
    ident: Ident,
    mutex_ident: Ident,
    ty: Type,
    vis: syn::Visibility,
    project_as_pin: bool,
}

#[proc_macro_attribute]
pub fn guarded(_args: TokenStream, input: TokenStream) -> TokenStream {
    let mut input_struct = parse_macro_input!(input as ItemStruct);

    let mut mutex_fields = Vec::new();
    let mut brwlock_fields = Vec::new();
    let mut guarded_fields = Vec::new();
    let mut errors = Vec::new();

    if let Fields::Named(ref mut fields) = input_struct.fields {
        for field in fields.named.iter_mut() {
            let mut is_mutex = false;
            let mut is_brwlock = false;
            let mut guarded_by = None;
            let mut is_pinned = false;
            let mut is_unpinned = false;
            let mut attrs_to_remove = Vec::new();
            let mut custom_class = None;
            let mut mutex_type = quote! { ::ksync::RawMutex };

            for (idx, attr) in field.attrs.iter().enumerate() {
                if attr.path().is_ident("mutex") {
                    is_mutex = true;
                    attrs_to_remove.push(idx);

                    if let syn::Meta::List(meta_list) = &attr.meta {
                        match meta_list.parse_args::<Ident>() {
                            Ok(ident) => {
                                custom_class = Some(ident);
                            }
                            Err(_) => {
                                errors.push(syn::Error::new(
                                    meta_list.span(),
                                    "#[mutex(LockClass)] accepts at most one identifier representing the lock class.",
                                ));
                            }
                        }
                    } else if !matches!(attr.meta, syn::Meta::Path(_)) {
                        errors.push(syn::Error::new(
                            attr.meta.span(),
                            "#[mutex] attribute must be either #[mutex] or #[mutex(LockClass)].",
                        ));
                    }
                } else if attr.path().is_ident("brwlock") {
                    is_brwlock = true;
                    attrs_to_remove.push(idx);
                } else if attr.path().is_ident("guarded_by") {
                    if let syn::Meta::List(meta_list) = &attr.meta {
                        if let Ok(ident) = meta_list.parse_args::<Ident>() {
                            guarded_by = Some(ident);
                        }
                    }
                    attrs_to_remove.push(idx);
                } else if attr.path().is_ident("pin") {
                    is_pinned = true;
                } else if attr.path().is_ident("ksync") {
                    if let syn::Meta::List(meta_list) = &attr.meta {
                        if let Ok(ident) = meta_list.parse_args::<Ident>() {
                            if ident == "unpinned" {
                                // TODO(https://fxbug.dev/536051491): Remove this workaround once
                                // WavlTree/region-alloc support safe pin projection.
                                is_unpinned = true;
                                attrs_to_remove.push(idx);
                            }
                        }
                    }
                }
            }

            // Remove our helper attributes in reverse order.
            for idx in attrs_to_remove.into_iter().rev() {
                field.attrs.remove(idx);
            }

            if is_unpinned && !is_wavltree_type(&field.ty) {
                errors.push(syn::Error::new(
                    field.ty.span(),
                    "The #[ksync(unpinned)] attribute is only allowed on WavlTree fields",
                ));
            }

            if is_mutex {
                if !is_kmutex_type(&field.ty) {
                    errors.push(syn::Error::new(
                        field.ty.span(),
                        "Mutex field must be of type KMutex",
                    ));
                }
                match extract_lock_type(&field.ty) {
                    Ok(Some(ty)) => {
                        mutex_type = ty;
                    }
                    Ok(None) => {}
                    Err(e) => {
                        errors.push(e);
                    }
                }
                // Automatically prepend the #[pin] attribute for pin-init layout
                field.attrs.push(syn::parse_quote!(#[pin]));
                mutex_fields.push((field.clone(), mutex_type, custom_class));
            } else if is_brwlock {
                if !is_brwlock_type(&field.ty) {
                    errors.push(syn::Error::new(
                        field.ty.span(),
                        "Brwlock field must be of type BrwLockPi",
                    ));
                }
                field.attrs.push(syn::parse_quote!(#[pin]));
                brwlock_fields.push(field.clone());
            } else if let Some(mutex_ident) = guarded_by {
                guarded_fields.push(GuardedField {
                    ident: field.ident.clone().unwrap(),
                    mutex_ident,
                    ty: field.ty.clone(),
                    vis: field.vis.clone(),
                    project_as_pin: is_pinned && !is_unpinned,
                });
            }
        }
    }

    if !errors.is_empty() {
        let compile_errors = errors.iter().map(|e| e.to_compile_error());
        return quote! { #(#compile_errors)* }.into();
    }

    // Automatically apply the #[::pin_init::pin_data] attribute to the parent struct
    // only if it doesn't already have one (e.g. #[pin_data(PinnedDrop)])
    let has_pin_data = input_struct
        .attrs
        .iter()
        .any(|attr| attr.path().segments.iter().any(|seg| seg.ident == "pin_data"));
    if !has_pin_data {
        input_struct.attrs.push(syn::parse_quote!(#[::pin_init::pin_data]));
    }

    let struct_ident = &input_struct.ident;
    let struct_vis = &input_struct.vis;

    let mut mutex_fields_processed = Vec::new();
    let mut brwlock_fields_processed = Vec::new();
    let mut generated_names = std::collections::HashSet::new();

    for (field, mutex_type, custom_class) in mutex_fields {
        let field_ident = field.ident.clone().unwrap();
        let mu_camel = to_camel_case(&field_ident.to_string());

        let guard_name = format!("{}{}Guard", struct_ident, mu_camel);

        if !generated_names.insert(guard_name.clone()) {
            errors.push(syn::Error::new(
                field_ident.span(),
                format!(
                    "The lock field '{}' generates the duplicate guard name '{}'. Please use distinct field names.",
                    field_ident, guard_name
                )
            ));
        }

        let class_ident = match custom_class {
            Some(ref ident) => ident.clone(),
            None => {
                let class_name = format!("{}{}", struct_ident, format_ident!("{}Class", mu_camel));
                if !generated_names.insert(class_name.clone()) {
                    errors.push(syn::Error::new(
                        field_ident.span(),
                        format!(
                            "The lock field '{}' generates the duplicate class name '{}'. Please use distinct field names.",
                            field_ident, class_name
                        )
                    ));
                }
                format_ident!("{}{}", struct_ident, format_ident!("{}Class", mu_camel))
            }
        };

        let is_phantom = is_phantom_mutex_type(&field.ty);

        mutex_fields_processed.push(MutexField {
            ident: field_ident,
            class_ident,
            mutex_type,
            custom_class,
            is_phantom,
        });
    }

    for field in brwlock_fields {
        let field_ident = field.ident.clone().unwrap();
        let mu_camel = to_camel_case(&field_ident.to_string());

        let class_name = format!("{}{}", struct_ident, format_ident!("{}Class", mu_camel));
        let read_guard_name = format!("{}{}ReadGuard", struct_ident, mu_camel);
        let write_guard_name = format!("{}{}WriteGuard", struct_ident, mu_camel);

        if !generated_names.insert(class_name.clone()) {
            errors.push(syn::Error::new(
                field_ident.span(),
                format!(
                    "The lock field '{}' generates the duplicate class name '{}'. Please use distinct field names.",
                    field_ident, class_name
                )
            ));
        }
        if !generated_names.insert(read_guard_name.clone()) {
            errors.push(syn::Error::new(
                field_ident.span(),
                format!(
                    "The lock field '{}' generates the duplicate guard name '{}'. Please use distinct field names.",
                    field_ident, read_guard_name
                )
            ));
        }
        if !generated_names.insert(write_guard_name.clone()) {
            errors.push(syn::Error::new(
                field_ident.span(),
                format!(
                    "The lock field '{}' generates the duplicate guard name '{}'. Please use distinct field names.",
                    field_ident, write_guard_name
                )
            ));
        }

        let class_ident = format_ident!("{}{}", struct_ident, format_ident!("{}Class", mu_camel));
        brwlock_fields_processed.push(MutexField {
            ident: field_ident,
            class_ident,
            mutex_type: quote! { ::ksync::RawBrwLockPi },
            custom_class: None,
            is_phantom: false,
        });
    }

    if !errors.is_empty() {
        let compile_errors = errors.iter().map(|e| e.to_compile_error());
        return quote! { #(#compile_errors)* }.into();
    }

    // Rewrite fields in the struct to be of type KMutex / KBrwLockPi and KCell
    if let Fields::Named(ref mut fields) = input_struct.fields {
        for field in fields.named.iter_mut() {
            let field_ident = field.ident.as_ref().unwrap();

            if let Some(mutex_field) =
                mutex_fields_processed.iter().find(|m| m.ident == *field_ident)
            {
                let class_ident = &mutex_field.class_ident;
                let mutex_type = &mutex_field.mutex_type;
                if let Type::Path(ref mut type_path) = field.ty {
                    if let Some(last_segment) = type_path.path.segments.last_mut() {
                        last_segment.ident = format_ident!("KMutex");
                        last_segment.arguments = syn::PathArguments::AngleBracketed(
                            syn::parse2(quote! { <#class_ident, #mutex_type> }).unwrap(),
                        );
                    }
                }
            } else if let Some(brwlock_field) =
                brwlock_fields_processed.iter().find(|m| m.ident == *field_ident)
            {
                let class_ident = &brwlock_field.class_ident;
                if let Type::Path(ref mut type_path) = field.ty {
                    if let Some(last_segment) = type_path.path.segments.last_mut() {
                        last_segment.ident = format_ident!("BrwLockPi");
                        last_segment.arguments = syn::PathArguments::AngleBracketed(
                            syn::parse2(quote! { <#class_ident> }).unwrap(),
                        );
                    }
                }
            } else if let Some(guarded_field) =
                guarded_fields.iter().find(|f| f.ident == *field_ident)
            {
                let class_ident_opt = mutex_fields_processed
                    .iter()
                    .chain(brwlock_fields_processed.iter())
                    .find(|m| m.ident == guarded_field.mutex_ident)
                    .map(|m| &m.class_ident);

                if let Some(class_ident) = class_ident_opt {
                    let original_ty = &field.ty;

                    field.ty = syn::parse2(quote! {
                        ::ksync::KCell<#original_ty, #class_ident>
                    })
                    .unwrap();
                    field.attrs.push(syn::parse_quote!(#[allow(dead_code)]));
                } else {
                    errors.push(syn::Error::new(
                        guarded_field.mutex_ident.span(),
                        format!(
                            "Guarded field '{}' references non-existent lock '{}'",
                            field_ident, guarded_field.mutex_ident
                        ),
                    ));
                }
            }
        }
    }

    if !errors.is_empty() {
        let compile_errors = errors.iter().map(|e| e.to_compile_error());
        return quote! { #(#compile_errors)* }.into();
    }

    let (impl_generics, ty_generics, where_clause) = input_struct.generics.split_for_impl();

    let ty_params: Vec<proc_macro2::TokenStream> = input_struct
        .generics
        .params
        .iter()
        .filter_map(|param| match param {
            syn::GenericParam::Type(type_param) => {
                let ident = &type_param.ident;
                Some(quote! { #ident })
            }
            syn::GenericParam::Const(const_param) => {
                let ident = &const_param.ident;
                Some(quote! { #ident })
            }
            syn::GenericParam::Lifetime(lifetime_param) => {
                let lifetime = &lifetime_param.lifetime;
                Some(quote! { #lifetime })
            }
        })
        .collect();

    let phantom_ty_params: Vec<&Ident> =
        input_struct.generics.type_params().map(|p| &p.ident).collect();

    let mut params_no_defaults = input_struct.generics.params.clone();
    for param in &mut params_no_defaults {
        match param {
            syn::GenericParam::Type(type_param) => {
                type_param.default = None;
            }
            syn::GenericParam::Const(const_param) => {
                const_param.default = None;
            }
            syn::GenericParam::Lifetime(_) => {}
        }
    }

    // 1. ZST Lock Class structures are shared globally
    let mut marker_structs = quote! {};
    for lock in mutex_fields_processed.iter().chain(brwlock_fields_processed.iter()) {
        if lock.custom_class.is_none() {
            let class_ident = &lock.class_ident;
            marker_structs.extend(quote! {
                #[allow(non_camel_case_types)]
                #struct_vis struct #class_ident;
            });
        }
    }

    let mut generated_code = quote! {};

    for mutex in &mutex_fields_processed {
        let mu_ident = &mutex.ident;
        let class_ident = &mutex.class_ident;
        let mutex_type = &mutex.mutex_type;

        let flags_expr = quote! { <#mutex_type as ::ksync::RawLock>::LOCK_FLAGS };

        let mu_camel = to_camel_case(&mu_ident.to_string());
        let guard_ident = format_ident!("{}{}Guard", struct_ident, mu_camel);
        let fields_ident = format_ident!("{}{}Fields", struct_ident, mu_camel);
        let fields_mut_ident = format_ident!("{}{}FieldsMut", struct_ident, mu_camel);
        let token_guard_ident = format_ident!("{}{}TokenGuard", struct_ident, mu_camel);
        let token_guard_mut_ident = format_ident!("{}{}TokenGuardMut", struct_ident, mu_camel);

        let mut lock_method_ident = format_ident!("lock_{}", mu_ident);
        lock_method_ident.set_span(mu_ident.span());
        let mut guard_method_ident = format_ident!("guard_{}", mu_ident);
        guard_method_ident.set_span(mu_ident.span());
        let mut guard_mut_method_ident = format_ident!("guard_{}_mut", mu_ident);
        guard_mut_method_ident.set_span(mu_ident.span());

        let this_guarded_fields: Vec<&GuardedField> =
            guarded_fields.iter().filter(|f| f.mutex_ident == *mu_ident).collect();

        let mut guard_accessors = quote! {};
        let mut fields_decl = quote! {};
        let mut fields_mut_decl = quote! {};
        let mut fields_init = quote! {};
        let mut fields_mut_init = quote! {};
        let mut token_guard_accessors = quote! {};
        let mut token_guard_mut_accessors = quote! {};

        for f in &this_guarded_fields {
            let f_ident = &f.ident;
            let f_ty = &f.ty;
            let f_vis = &f.vis;

            let f_mut_ident = format_ident!("{}_mut", f_ident);

            token_guard_accessors.extend(quote! {
                #[inline]
                #f_vis fn #f_ident(&self) -> &#f_ty {
                    // SAFETY: The lock token proves that the lock protecting this cell is held.
                    unsafe { self.parent.#f_ident.get(self.token) }
                }
            });

            if f.project_as_pin {
                token_guard_mut_accessors.extend(quote! {
                    #[inline]
                    #f_vis fn #f_ident(&self) -> &#f_ty {
                        // SAFETY: The lock token proves that the lock protecting this cell is held.
                        unsafe { self.parent.#f_ident.get(&*self.token) }
                    }

                    #[inline]
                    #f_vis fn #f_mut_ident(&mut self) -> ::core::pin::Pin<&mut #f_ty> {
                        // SAFETY: We hold an exclusive mutable reference to the guard token,
                        // and the parent struct is pinned so structurally pinned fields remain pinned.
                        unsafe { ::core::pin::Pin::new_unchecked(self.parent.#f_ident.get_mut(&mut *self.token)) }
                    }
                });
            } else {
                token_guard_mut_accessors.extend(quote! {
                    #[inline]
                    #f_vis fn #f_ident(&self) -> &#f_ty {
                        // SAFETY: The lock token proves that the lock protecting this cell is held.
                        unsafe { self.parent.#f_ident.get(&*self.token) }
                    }

                    #[inline]
                    #f_vis fn #f_mut_ident(&mut self) -> &mut #f_ty {
                        // SAFETY: We hold an exclusive mutable reference to the guard token.
                        unsafe { self.parent.#f_ident.get_mut(&mut *self.token) }
                    }
                });
            }

            if f.project_as_pin {
                guard_accessors.extend(quote! {
                    #[inline]
                    #f_vis fn #f_ident(&self) -> &#f_ty {
                        // SAFETY: The lock token proves that the lock protecting this cell is held.
                        unsafe { self.parent.#f_ident.get(self.inner.token()) }
                    }

                    #[inline]
                    #f_vis fn #f_mut_ident(self: ::core::pin::Pin<&mut Self>) -> ::core::pin::Pin<&mut #f_ty> {
                        // SAFETY: Safe projection to obtain pinned reference to self
                        // without moving fields.
                        let me = unsafe { self.get_unchecked_mut() };
                        // SAFETY: `inner` is structurally pinned inside the pinned guard `self`.
                        let inner_pin = unsafe { ::core::pin::Pin::new_unchecked(&mut me.inner) };
                        // SAFETY: We hold an exclusive mutable reference to the guard token,
                        // and the parent is pinned.
                        unsafe { ::core::pin::Pin::new_unchecked(me.parent.#f_ident.get_mut(inner_pin.token_mut())) }
                    }
                });

                fields_mut_decl.extend(quote! {
                    #[allow(dead_code)]
                    #f_vis #f_ident: ::core::pin::Pin<&'b mut #f_ty>,
                });

                fields_mut_init.extend(quote! {
                    // SAFETY: We hold exclusive access to the guard token, each field cell
                    // is disjoint, and the parent struct is pinned so structurally pinned
                    // fields remain pinned.
                    #f_ident: unsafe { ::core::pin::Pin::new_unchecked(&mut *me.parent.#f_ident.as_mut_ptr(token)) },
                });
            } else {
                guard_accessors.extend(quote! {
                    #[inline]
                    #f_vis fn #f_ident(&self) -> &#f_ty {
                        // SAFETY: The lock token proves that the lock protecting this cell is held.
                        unsafe { self.parent.#f_ident.get(self.inner.token()) }
                    }

                    #[inline]
                    #f_vis fn #f_mut_ident(self: ::core::pin::Pin<&mut Self>) -> &mut #f_ty {
                        // SAFETY: Safe projection to obtain unpinned reference to self
                        // without moving fields.
                        let me = unsafe { self.get_unchecked_mut() };
                        // SAFETY: `inner` is structurally pinned inside the pinned guard `self`.
                        let inner_pin = unsafe { ::core::pin::Pin::new_unchecked(&mut me.inner) };
                        // SAFETY: We hold an exclusive mutable reference to the guard token.
                        unsafe { me.parent.#f_ident.get_mut(inner_pin.token_mut()) }
                    }
                });

                fields_mut_decl.extend(quote! {
                    #[allow(dead_code)]
                    #f_vis #f_ident: &'b mut #f_ty,
                });

                fields_mut_init.extend(quote! {
                    // SAFETY: We hold exclusive access to the guard token and each field cell
                    // is disjoint.
                    #f_ident: unsafe { &mut *me.parent.#f_ident.as_mut_ptr(token) },
                });
            }

            fields_decl.extend(quote! {
                #[allow(dead_code)]
                #f_vis #f_ident: &'b #f_ty,
            });

            fields_init.extend(quote! {
                // SAFETY: The guard token proves shared access to the cell.
                #f_ident: unsafe { me.parent.#f_ident.get(token) },
            });
        }

        let params_with_bounds = &params_no_defaults;
        let fields_decl_generics = quote! { <'b, #params_with_bounds> };
        let fields_ty_generics = quote! { <'b, #(#ty_params),*> };
        let fields_ret_ty_generics = quote! { <'f, #(#ty_params),*> };
        let token_guard_decl_generics = quote! { <'b, 'a, #params_with_bounds> };
        let token_guard_impl_generics = quote! { <'b, 'a, #params_with_bounds> };
        let token_guard_ty_generics = quote! { <'b, 'a, #(#ty_params),*> };

        let struct_upper = struct_ident.to_string().to_ascii_uppercase();
        let mu_upper = mu_camel.to_string().to_ascii_uppercase();
        let reg_ident = format_ident!("{}_{}_REGISTRATION", struct_upper, mu_upper);
        let string_reg_ident = format_ident!("{}_{}_STRING_REG", struct_upper, mu_upper);
        let path_name = format!("{}::{}", struct_ident, mu_ident);

        let guard_struct_generics = if mutex.is_phantom {
            let mut params = params_no_defaults.clone();
            params.push(syn::parse_quote!(M: ::ksync::RawLock = ::ksync::RawMutex));
            quote! { <'a, #params> }
        } else {
            quote! { <'a, #params_with_bounds> }
        };
        let guard_impl_generics = if mutex.is_phantom {
            let mut params = params_no_defaults.clone();
            params.push(syn::parse_quote!(M: ::ksync::RawLock));
            quote! { <'a, #params> }
        } else {
            quote! { <'a, #params_with_bounds> }
        };
        let guard_ty_generics = if mutex.is_phantom {
            let mut args = ty_params.clone();
            args.push(quote! { M });
            quote! { <'a, #(#args),*> }
        } else {
            quote! { <'a, #(#ty_params),*> }
        };
        let guard_inner_type = if mutex.is_phantom {
            quote! { ::ksync::KMutexGuard<'a, #class_ident, M> }
        } else {
            quote! { ::ksync::KMutexGuard<'a, #class_ident, #mutex_type> }
        };
        let lock_method_def = if mutex.is_phantom {
            quote! {
                #[inline]
                #struct_vis fn #lock_method_ident<'a, M: ::ksync::RawLock>(
                    &'a self,
                    real_mutex: &'a ::ksync::KMutex<#class_ident, M>,
                ) -> impl pin_init::PinInit<#guard_ident #guard_ty_generics, ::core::convert::Infallible> {
                    pin_init::pin_init!(#guard_ident {
                        parent: self,
                        inner <- ::ksync::KMutexGuard::new(real_mutex),
                    })
                }
            }
        } else {
            let return_ty_generics = quote! { <'_, #(#ty_params),*> };
            quote! {
                #[inline]
                #struct_vis fn #lock_method_ident(&self) -> impl pin_init::PinInit<#guard_ident #return_ty_generics, ::core::convert::Infallible> {
                    pin_init::pin_init!(#guard_ident {
                        parent: self,
                        inner <- ::ksync::KMutexGuard::new(&self.#mu_ident),
                    })
                }
            }
        };

        let guard_method_def = quote! {
            #[inline]
            #struct_vis fn #guard_method_ident<'b, 'a>(
                &'b self,
                token: &'b ::ksync::LockToken<'a, #class_ident>,
            ) -> #token_guard_ident #token_guard_ty_generics {
                #token_guard_ident {
                    parent: self,
                    token,
                }
            }

            #[inline]
            #struct_vis fn #guard_mut_method_ident<'b, 'a>(
                &'b self,
                token: &'b mut ::ksync::LockToken<'a, #class_ident>,
            ) -> #token_guard_mut_ident #token_guard_ty_generics {
                #token_guard_mut_ident {
                    parent: self,
                    token,
                }
            }
        };

        let class_registration_code = if mutex.custom_class.is_none() {
            quote! {
                ::ksync::declare_interned_string!(#string_reg_ident, #path_name);

                #[unsafe(link_section = "rust_lock_classes")]
                #[used]
                static #reg_ident: ::ksync::LockClassRegistration = ::ksync::LockClassRegistration::with_flags(
                    #string_reg_ident,
                    #flags_expr,
                );

                impl ::ksync::LockClass for #class_ident {
                    const ID: *mut ::core::ffi::c_void = #reg_ident.get();
                }
            }
        } else {
            quote! {}
        };

        generated_code.extend(quote! {
            #[allow(dead_code)]
            #struct_vis struct #fields_ident #fields_decl_generics #where_clause {
                #fields_decl
                _marker: ::core::marker::PhantomData<(&'b (), #(#phantom_ty_params),*)>,
            }

            #[allow(dead_code)]
            #struct_vis struct #fields_mut_ident #fields_decl_generics #where_clause {
                #fields_mut_decl
                _marker: ::core::marker::PhantomData<(&'b (), #(#phantom_ty_params),*)>,
            }

            #class_registration_code

            #[pin_init::pin_data(PinnedDrop)]
            #struct_vis struct #guard_ident #guard_struct_generics #where_clause {
                parent: &'a #struct_ident #ty_generics,
                #[pin]
                inner: #guard_inner_type,
            }

            #[pin_init::pinned_drop]
            impl #guard_impl_generics pin_init::PinnedDrop for #guard_ident #guard_ty_generics #where_clause {
                fn drop(self: ::core::pin::Pin<&mut Self>) {
                }
            }

            impl #guard_impl_generics #guard_ident #guard_ty_generics #where_clause {
                #guard_accessors

                #[inline]
                #struct_vis fn token(&self) -> &::ksync::LockToken<'a, #class_ident> {
                    self.inner.token()
                }

                #[inline]
                #struct_vis fn token_mut(self: ::core::pin::Pin<&mut Self>) -> &mut ::ksync::LockToken<'a, #class_ident> {
                    // SAFETY: Safe projection to obtain unpinned reference to self without moving fields.
                    let me = unsafe { self.get_unchecked_mut() };
                    // SAFETY: `inner` is structurally pinned inside `self`.
                    let inner_pin = unsafe { ::core::pin::Pin::new_unchecked(&mut me.inner) };
                    inner_pin.token_mut()
                }

                #[inline]
                #struct_vis fn fields<'b>(&'b self) -> #fields_ident #fields_ty_generics {
                    let me = self;
                    let token = me.inner.token();
                    #fields_ident {
                        #fields_init
                        _marker: ::core::marker::PhantomData,
                    }
                }

                #[inline]
                #struct_vis fn fields_mut<'b>(self: ::core::pin::Pin<&'b mut Self>) -> #fields_mut_ident #fields_ty_generics {
                    // SAFETY: Safe projection to obtain unpinned reference to self without moving fields.
                    let me = unsafe { self.get_unchecked_mut() };
                    // SAFETY: `inner` is structurally pinned inside `self`.
                    let inner_pin = unsafe { ::core::pin::Pin::new_unchecked(&mut me.inner) };
                    let token = inner_pin.token_mut();
                    #fields_mut_ident {
                        #fields_mut_init
                        _marker: ::core::marker::PhantomData,
                    }
                }
            }

            #[allow(dead_code)]
            #struct_vis struct #token_guard_ident #token_guard_decl_generics #where_clause {
                parent: &'b #struct_ident #ty_generics,
                token: &'b ::ksync::LockToken<'a, #class_ident>,
            }

            impl #token_guard_impl_generics #token_guard_ident #token_guard_ty_generics #where_clause {
                #token_guard_accessors

                #[inline]
                #struct_vis fn token(&self) -> &::ksync::LockToken<'a, #class_ident> {
                    self.token
                }

                #[inline]
                #struct_vis fn fields<'f>(&'f self) -> #fields_ident #fields_ret_ty_generics {
                    let me = self;
                    let token = me.token;
                    #fields_ident {
                        #fields_init
                        _marker: ::core::marker::PhantomData,
                    }
                }
            }

            #[allow(dead_code)]
            #struct_vis struct #token_guard_mut_ident #token_guard_decl_generics #where_clause {
                parent: &'b #struct_ident #ty_generics,
                token: &'b mut ::ksync::LockToken<'a, #class_ident>,
            }

            impl #token_guard_impl_generics #token_guard_mut_ident #token_guard_ty_generics #where_clause {
                #token_guard_mut_accessors

                #[inline]
                #struct_vis fn token(&self) -> &::ksync::LockToken<'a, #class_ident> {
                    &*self.token
                }

                #[inline]
                #struct_vis fn token_mut(&mut self) -> &mut ::ksync::LockToken<'a, #class_ident> {
                    &mut *self.token
                }

                #[inline]
                #struct_vis fn fields<'f>(&'f self) -> #fields_ident #fields_ret_ty_generics {
                    let me = self;
                    let token = &*me.token;
                    #fields_ident {
                        #fields_init
                        _marker: ::core::marker::PhantomData,
                    }
                }

                #[inline]
                #struct_vis fn fields_mut<'f>(&'f mut self) -> #fields_mut_ident #fields_ret_ty_generics {
                    let me = self;
                    let token = &mut *me.token;
                    #fields_mut_ident {
                        #fields_mut_init
                        _marker: ::core::marker::PhantomData,
                    }
                }
            }

            impl #impl_generics #struct_ident #ty_generics #where_clause {
                #lock_method_def
                #guard_method_def
            }
        });
    }

    for brwlock in &brwlock_fields_processed {
        let lock_ident = &brwlock.ident;
        let class_ident = &brwlock.class_ident;

        let mu_camel = to_camel_case(&lock_ident.to_string());
        let read_guard_ident = format_ident!("{}{}ReadGuard", struct_ident, mu_camel);
        let write_guard_ident = format_ident!("{}{}WriteGuard", struct_ident, mu_camel);
        let read_fields_ident = format_ident!("{}{}ReadFields", struct_ident, mu_camel);
        let write_fields_ident = format_ident!("{}{}WriteFields", struct_ident, mu_camel);

        let read_token_guard_ident = format_ident!("{}{}ReadTokenGuard", struct_ident, mu_camel);
        let write_token_guard_ident = format_ident!("{}{}WriteTokenGuard", struct_ident, mu_camel);
        let mut read_lock_method_ident = format_ident!("read_{}", lock_ident);
        read_lock_method_ident.set_span(lock_ident.span());
        let mut write_lock_method_ident = format_ident!("write_{}", lock_ident);
        write_lock_method_ident.set_span(lock_ident.span());
        let mut guard_read_method_ident = format_ident!("guard_read_{}", lock_ident);
        guard_read_method_ident.set_span(lock_ident.span());
        let mut guard_write_method_ident = format_ident!("guard_write_{}", lock_ident);
        guard_write_method_ident.set_span(lock_ident.span());

        let this_guarded_fields: Vec<&GuardedField> =
            guarded_fields.iter().filter(|f| f.mutex_ident == *lock_ident).collect();

        let mut read_guard_accessors = quote! {};
        let mut write_guard_accessors = quote! {};
        let mut read_fields_decl = quote! {};
        let mut write_fields_decl = quote! {};
        let mut read_fields_init = quote! {};
        let mut write_fields_init = quote! {};
        let mut read_token_guard_accessors = quote! {};
        let mut write_token_guard_accessors = quote! {};

        for f in &this_guarded_fields {
            let f_ident = &f.ident;
            let f_ty = &f.ty;
            let f_vis = &f.vis;

            let f_mut_ident = format_ident!("{}_mut", f_ident);

            read_token_guard_accessors.extend(quote! {
                #[inline]
                #f_vis fn #f_ident(&self) -> &#f_ty {
                    // SAFETY: The read token proves shared access to this cell.
                    unsafe { self.parent.#f_ident.get(self.token) }
                }
            });

            if f.project_as_pin {
                write_token_guard_accessors.extend(quote! {
                    #[inline]
                    #f_vis fn #f_ident(&self) -> &#f_ty {
                        // SAFETY: The write token proves shared access to this cell.
                        unsafe { self.parent.#f_ident.get(&*self.token) }
                    }

                    #[inline]
                    #f_vis fn #f_mut_ident(&mut self) -> ::core::pin::Pin<&mut #f_ty> {
                        // SAFETY: We hold an exclusive mutable reference to the write token,
                        // and the parent struct is pinned so structurally pinned fields remain pinned.
                        unsafe { ::core::pin::Pin::new_unchecked(self.parent.#f_ident.get_mut(&mut *self.token)) }
                    }
                });
            } else {
                write_token_guard_accessors.extend(quote! {
                    #[inline]
                    #f_vis fn #f_ident(&self) -> &#f_ty {
                        // SAFETY: The write token proves shared access to this cell.
                        unsafe { self.parent.#f_ident.get(&*self.token) }
                    }

                    #[inline]
                    #f_vis fn #f_mut_ident(&mut self) -> &mut #f_ty {
                        // SAFETY: We hold an exclusive mutable reference to the write token.
                        unsafe { self.parent.#f_ident.get_mut(&mut *self.token) }
                    }
                });
            }

            read_guard_accessors.extend(quote! {
                #[inline]
                #f_vis fn #f_ident(&self) -> &#f_ty {
                    // SAFETY: The read guard token proves shared access to this cell.
                    unsafe { self.parent.#f_ident.get(self.inner.token()) }
                }
            });

            write_guard_accessors.extend(quote! {
                #[inline]
                #f_vis fn #f_ident(&self) -> &#f_ty {
                    // SAFETY: The write guard token proves shared access to this cell.
                    unsafe { self.parent.#f_ident.get(self.inner.token()) }
                }

                #[inline]
                #f_vis fn #f_mut_ident(self: ::core::pin::Pin<&mut Self>) -> &mut #f_ty {
                    // SAFETY: Safe projection to obtain unpinned reference to self without moving fields.
                    let me = unsafe { self.get_unchecked_mut() };
                    // SAFETY: `inner` is structurally pinned inside the pinned guard `self`.
                    let inner_pin = unsafe { ::core::pin::Pin::new_unchecked(&mut me.inner) };
                    // SAFETY: We hold an exclusive mutable reference to the write guard token.
                    unsafe { me.parent.#f_ident.get_mut(inner_pin.token_mut()) }
                }
            });

            read_fields_decl.extend(quote! {
                #[allow(dead_code)]
                #f_vis #f_ident: &'b #f_ty,
            });

            write_fields_decl.extend(quote! {
                #[allow(dead_code)]
                #f_vis #f_ident: &'b mut #f_ty,
            });

            read_fields_init.extend(quote! {
                // SAFETY: The guard token proves shared access to the cell.
                #f_ident: unsafe { me.parent.#f_ident.get(token) },
            });

            write_fields_init.extend(quote! {
                // SAFETY: We hold exclusive access to the write guard token and each field cell is disjoint.
                #f_ident: unsafe { &mut *me.parent.#f_ident.as_mut_ptr(token) },
            });
        }

        let params_with_bounds = &params_no_defaults;
        let guard_decl_generics = quote! { <'a, #params_with_bounds> };
        let guard_ty_generics = quote! { <'a, #(#ty_params),*> };
        let return_ty_generics = quote! { <'_, #(#ty_params),*> };
        let token_guard_decl_generics = quote! { <'b, 'a, #params_with_bounds> };
        let token_guard_impl_generics = quote! { <'b, 'a, #params_with_bounds> };
        let token_guard_ty_generics = quote! { <'b, 'a, #(#ty_params),*> };
        let fields_decl_generics = quote! { <'b, #params_with_bounds> };
        let fields_ty_generics = quote! { <'b, #(#ty_params),*> };
        let fields_ret_ty_generics = quote! { <'f, #(#ty_params),*> };

        let struct_upper = struct_ident.to_string().to_ascii_uppercase();
        let mu_upper = mu_camel.to_string().to_ascii_uppercase();
        let reg_ident = format_ident!("{}_{}_REGISTRATION", struct_upper, mu_upper);
        let string_reg_ident = format_ident!("{}_{}_STRING_REG", struct_upper, mu_upper);
        let path_name = format!("{}::{}", struct_ident, lock_ident);

        generated_code.extend(quote! {
            #[allow(dead_code)]
            #struct_vis struct #read_fields_ident #fields_decl_generics #where_clause {
                #read_fields_decl
                _marker: ::core::marker::PhantomData<(&'b (), #(#phantom_ty_params),*)>,
            }

            #[allow(dead_code)]
            #struct_vis struct #write_fields_ident #fields_decl_generics #where_clause {
                #write_fields_decl
                _marker: ::core::marker::PhantomData<(&'b (), #(#phantom_ty_params),*)>,
            }

            ::ksync::declare_interned_string!(#string_reg_ident, #path_name);

            #[unsafe(link_section = "rust_lock_classes")]
            #[used]
            static #reg_ident: ::ksync::LockClassRegistration = ::ksync::LockClassRegistration::new(&#string_reg_ident);

            impl ::ksync::LockClass for #class_ident {
                const ID: *mut ::core::ffi::c_void = #reg_ident.get();
            }

            #[pin_init::pin_data(PinnedDrop)]
            #struct_vis struct #read_guard_ident #guard_decl_generics #where_clause {
                parent: &'a #struct_ident #ty_generics,
                #[pin]
                inner: ::ksync::BrwLockPiReadGuard<'a, #class_ident>,
            }

            #[pin_init::pinned_drop]
            impl #guard_decl_generics pin_init::PinnedDrop for #read_guard_ident #guard_ty_generics #where_clause {
                fn drop(self: ::core::pin::Pin<&mut Self>) {
                }
            }

            impl #guard_decl_generics #read_guard_ident #guard_ty_generics #where_clause {
                #read_guard_accessors

                #[inline]
                #struct_vis fn fields<'b>(&'b self) -> #read_fields_ident #fields_ty_generics {
                    let me = self;
                    let token = me.inner.token();
                    #read_fields_ident {
                        #read_fields_init
                        _marker: ::core::marker::PhantomData,
                    }
                }
            }

            #[pin_init::pin_data(PinnedDrop)]
            #struct_vis struct #write_guard_ident #guard_decl_generics #where_clause {
                parent: &'a #struct_ident #ty_generics,
                #[pin]
                inner: ::ksync::BrwLockPiWriteGuard<'a, #class_ident>,
            }

            #[pin_init::pinned_drop]
            impl #guard_decl_generics pin_init::PinnedDrop for #write_guard_ident #guard_ty_generics #where_clause {
                fn drop(self: ::core::pin::Pin<&mut Self>) {
                }
            }

            impl #guard_decl_generics #write_guard_ident #guard_ty_generics #where_clause {
                #write_guard_accessors

                #[inline]
                #struct_vis fn fields<'b>(&'b self) -> #read_fields_ident #fields_ty_generics {
                    let me = self;
                    let token = me.inner.token();
                    #read_fields_ident {
                        #read_fields_init
                        _marker: ::core::marker::PhantomData,
                    }
                }

                #[inline]
                #struct_vis fn fields_mut<'b>(self: ::core::pin::Pin<&'b mut Self>) -> #write_fields_ident #fields_ty_generics {
                    // SAFETY: Safe projection to obtain unpinned reference to self without moving fields.
                    let me = unsafe { self.get_unchecked_mut() };
                    // SAFETY: `inner` is structurally pinned inside `self`.
                    let inner_pin = unsafe { ::core::pin::Pin::new_unchecked(&mut me.inner) };
                    let token = inner_pin.token_mut();
                    #write_fields_ident {
                        #write_fields_init
                        _marker: ::core::marker::PhantomData,
                    }
                }
            }

            #[allow(dead_code)]
            #struct_vis struct #read_token_guard_ident #token_guard_decl_generics #where_clause {
                parent: &'b #struct_ident #ty_generics,
                token: &'b ::ksync::LockToken<'a, #class_ident>,
            }

            impl #token_guard_impl_generics #read_token_guard_ident #token_guard_ty_generics #where_clause {
                #read_token_guard_accessors

                #[inline]
                #struct_vis fn token(&self) -> &::ksync::LockToken<'a, #class_ident> {
                    self.token
                }

                #[inline]
                #struct_vis fn fields<'f>(&'f self) -> #read_fields_ident #fields_ret_ty_generics {
                    let me = self;
                    let token = me.token;
                    #read_fields_ident {
                        #read_fields_init
                        _marker: ::core::marker::PhantomData,
                    }
                }
            }

            #[allow(dead_code)]
            #struct_vis struct #write_token_guard_ident #token_guard_decl_generics #where_clause {
                parent: &'b #struct_ident #ty_generics,
                token: &'b mut ::ksync::LockToken<'a, #class_ident>,
            }

            impl #token_guard_impl_generics #write_token_guard_ident #token_guard_ty_generics #where_clause {
                #write_token_guard_accessors

                #[inline]
                #struct_vis fn token(&self) -> &::ksync::LockToken<'a, #class_ident> {
                    &*self.token
                }

                #[inline]
                #struct_vis fn token_mut(&mut self) -> &mut ::ksync::LockToken<'a, #class_ident> {
                    &mut *self.token
                }

                #[inline]
                #struct_vis fn fields<'f>(&'f self) -> #read_fields_ident #fields_ret_ty_generics {
                    let me = self;
                    let token = &*me.token;
                    #read_fields_ident {
                        #read_fields_init
                        _marker: ::core::marker::PhantomData,
                    }
                }

                #[inline]
                #struct_vis fn fields_mut<'f>(&'f mut self) -> #write_fields_ident #fields_ret_ty_generics {
                    let me = self;
                    let token = &mut *me.token;
                    #write_fields_ident {
                        #write_fields_init
                        _marker: ::core::marker::PhantomData,
                    }
                }
            }

            impl #impl_generics #struct_ident #ty_generics #where_clause {
                #[inline]
                #struct_vis fn #read_lock_method_ident(&self) -> impl pin_init::PinInit<#read_guard_ident #return_ty_generics, ::core::convert::Infallible> {
                    pin_init::pin_init!(#read_guard_ident {
                        parent: self,
                        inner <- ::ksync::BrwLockPiReadGuard::new(&self.#lock_ident),
                    })
                }

                #[inline]
                #struct_vis fn #write_lock_method_ident(&self) -> impl pin_init::PinInit<#write_guard_ident #return_ty_generics, ::core::convert::Infallible> {
                    pin_init::pin_init!(#write_guard_ident {
                        parent: self,
                        inner <- ::ksync::BrwLockPiWriteGuard::new(&self.#lock_ident),
                    })
                }

                #[inline]
                #struct_vis fn #guard_read_method_ident<'b, 'a>(
                    &'b self,
                    token: &'b ::ksync::LockToken<'a, #class_ident>,
                ) -> #read_token_guard_ident #token_guard_ty_generics {
                    #read_token_guard_ident {
                        parent: self,
                        token,
                    }
                }

                #[inline]
                #struct_vis fn #guard_write_method_ident<'b, 'a>(
                    &'b self,
                    token: &'b mut ::ksync::LockToken<'a, #class_ident>,
                ) -> #write_token_guard_ident #token_guard_ty_generics {
                    #write_token_guard_ident {
                        parent: self,
                        token,
                    }
                }
            }
        });
    }

    let expanded = quote! {
        #marker_structs
        #input_struct
        #generated_code
    };

    TokenStream::from(expanded)
}

fn to_camel_case(s: &str) -> String {
    let mut camel = String::new();
    let mut capitalize = true;
    for c in s.chars() {
        if c == '_' {
            capitalize = true;
        } else if capitalize {
            camel.push(c.to_ascii_uppercase());
            capitalize = false;
        } else {
            camel.push(c);
        }
    }
    camel
}

fn is_wavltree_type(ty: &Type) -> bool {
    if let Type::Path(TypePath { path, .. }) = ty {
        path.segments.iter().any(|seg| seg.ident == "WavlTree")
    } else {
        false
    }
}

fn is_kmutex_type(ty: &Type) -> bool {
    if let Type::Path(TypePath { path, .. }) = ty {
        path.segments.iter().any(|seg| seg.ident == "KMutex")
    } else {
        false
    }
}

fn is_brwlock_type(ty: &Type) -> bool {
    if let Type::Path(TypePath { path, .. }) = ty {
        path.segments.iter().any(|seg| seg.ident == "BrwLockPi")
    } else {
        false
    }
}

fn extract_lock_type(ty: &Type) -> Result<Option<proc_macro2::TokenStream>, syn::Error> {
    if let Type::Path(type_path) = ty {
        if let Some(last_segment) = type_path.path.segments.last() {
            if last_segment.ident == "KMutex" {
                match &last_segment.arguments {
                    syn::PathArguments::None => return Ok(None),
                    syn::PathArguments::AngleBracketed(args) => {
                        if args.args.len() == 1 {
                            let first_arg = &args.args[0];
                            return Ok(Some(quote! { #first_arg }));
                        } else if args.args.is_empty() {
                            return Ok(None);
                        } else {
                            return Err(syn::Error::new(
                                args.span(),
                                "KMutex expects at most 1 generic argument for the lock type in struct definition",
                            ));
                        }
                    }
                    syn::PathArguments::Parenthesized(args) => {
                        return Err(syn::Error::new(
                            args.span(),
                            "KMutex does not support parenthesized generic arguments",
                        ));
                    }
                }
            }
        }
    }
    Ok(None)
}

fn is_phantom_mutex_type(ty: &Type) -> bool {
    if let Type::Path(type_path) = ty {
        if let Some(last_segment) = type_path.path.segments.last() {
            if last_segment.ident == "KMutex" {
                if let syn::PathArguments::AngleBracketed(args) = &last_segment.arguments {
                    if let Some(syn::GenericArgument::Type(Type::Path(arg_path))) =
                        args.args.first()
                    {
                        if let Some(arg_seg) = arg_path.path.segments.last() {
                            return arg_seg.ident == "PhantomMutex";
                        }
                    }
                }
            }
        }
    }
    false
}
