use crate::vrm::Vrm;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

#[derive(SystemParam)]
pub struct ParentSearcher<'w, 's> {
    entities: Query<'w, 's, (Option<&'static ChildOf>, Has<Vrm>)>,
}

impl ParentSearcher<'_, '_> {
    #[inline]
    pub fn find_vrm(
        &self,
        source: Entity,
    ) -> Option<Entity> {
        find_entity(source, &self.entities)
    }
}

fn find_entity(
    entity: Entity,
    entities: &Query<(Option<&ChildOf>, Has<Vrm>)>,
) -> Option<Entity> {
    let (child_of, has_vrm) = entities.get(entity).ok()?;

    if has_vrm {
        return Some(entity);
    }
    if let Some(parent) = child_of {
        return find_entity(parent.0, entities);
    }
    None
}

#[cfg(test)]
mod tests {
    use crate::prelude::*;
    use crate::system_param::parent_searcher::ParentSearcher;
    use crate::tests::test_app;
    use bevy::ecs::system::RunSystemOnce;
    use bevy_test_helper::system::SystemExt;

    #[test]
    fn test_find_vrm() {
        let mut app = test_app();

        let vrm = app.world_mut().spawn(Vrm).id();
        let child = app.world_mut().spawn_empty().id();
        app.world_mut().commands().entity(vrm).add_child(child);
        app.update();

        let actual = app
            .world_mut()
            .run_system_once(move |s: ParentSearcher| s.find_vrm(child))
            .unwrap();
        assert_eq!(actual, Some(vrm));
    }

    #[test]
    fn test_find_vrm_with_root() {
        let mut app = test_app();

        let root = app.world_mut().spawn_empty().id();
        let vrm = app.world_mut().spawn(Vrm).id();
        app.world_mut().commands().entity(root).add_child(vrm);
        let child = app.world_mut().spawn_empty().id();
        app.world_mut().commands().entity(vrm).add_child(child);
        app.update();

        let actual = app
            .world_mut()
            .run_system_once(move |s: ParentSearcher| s.find_vrm(child))
            .unwrap();
        assert_eq!(actual, Some(vrm));
    }
}
