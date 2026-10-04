#[macro_export]
macro_rules! new_type {
    (
        $(#[$meta:meta])*
        name: $struct_name: ident,
        ty: String,
    ) => {
        $(#[$meta])*
        #[derive(
            bevy::prelude::Component,
            bevy::prelude::Reflect,
            Ord,
            PartialOrd,
            Eq,
            PartialEq,
            Debug,
            Clone,
            Hash,
            bevy::prelude::Deref,
        )]
        #[reflect(Component)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
        #[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
        pub struct $struct_name(pub String);

        impl From<&str> for $struct_name {
            fn from(value: &str) -> Self {
                Self(value.to_string())
            }
        }

        impl std::fmt::Display for $struct_name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
    (
        $(#[$meta:meta])*
        name: $struct_name: ident,
        ty: $ty: ident,
    ) => {
        #[derive(
            bevy::prelude::Component,
            Eq,
            PartialEq,
            Debug,
            serde::Serialize,
            serde::Deserialize,
            Copy,
            Clone,
            Hash,
            bevy::prelude::Deref,
        )]
        pub struct $struct_name(pub $ty);

        impl From<$ty> for $struct_name {
            fn from(value: $ty) -> Self {
                Self(value)
            }
        }
    };
}

macro_rules! marker_component {
        (
            $(#[$meta:meta])*
            $name: ident
        ) => {
            $(#[$meta])*
            #[derive(
                Component,
                Default,
                Debug,
                Copy,
                Clone,
                Eq,
                PartialEq,
                Hash,
                Reflect,
            )]
            #[reflect(Component, Default)]
            #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
            #[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
            pub struct $name;
        };
    }

macro_rules! entity_component {
        (
            $(#[$meta:meta])*
            $name: ident
        ) => {
            $(#[$meta])*
            #[derive(
                Component,
                Debug,
                Copy,
                Clone,
                Eq,
                PartialEq,
                Hash,
                Reflect,
                bevy::prelude::Deref,
            )]
            #[reflect(Component)]
            #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
            #[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
            // `#[entities]` is mandatory, not decoration: `#[derive(Component)]`
            // turns every `#[entities]` field into a `Component::map_entities`
            // call on that field's type
            // (`bevy_ecs_macro_logic/src/map_entities.rs:37-51`, wrapped in the
            // `use bevy_ecs::entity::MapEntities` that makes it resolve at
            // `bevy_ecs_macro_logic/src/component.rs:199-214`). That is the only
            // hook the scene spawn pipeline consults: instantiating a
            // `WorldAsset` runs `ReflectComponent::apply_or_insert_mapped`, which
            // calls `C::map_entities` for every component it copies
            // (`bevy_world_serialization/src/world_asset.rs:191-199` ->
            // `bevy_ecs/src/reflect/component.rs:340`, `:345`, `:353`).
            //
            // `Entity: MapEntities` is bevy's own impl
            // (`bevy_ecs/src/entity/map_entities.rs:62-66`), so the field needs
            // nothing else. Omit the attribute and an instantiated avatar keeps
            // the loader's scratch-world id: gaze control, body tracking and the
            // first-person auto split then read a stale bone, or worse, whatever
            // unrelated entity occupies that id. See `crate::vrm::components`.
            pub struct $name(#[entities] pub bevy::prelude::Entity);
        };
    }

pub(crate) use entity_component;
pub(crate) use marker_component;
