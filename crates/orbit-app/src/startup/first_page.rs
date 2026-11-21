use std::collections::HashMap;

use gpui::{App, EntityId, Global, WeakEntity, Window, WindowId};

use crate::view::AppView;

#[derive(Clone, Hash, PartialEq, Eq)]
pub(crate) enum Asset {
    Preview(EntityId),
    Media(EntityId, String),
}

pub(crate) struct FirstPage {
    owner: WeakEntity<AppView>,
    window: WindowId,
    assets: HashMap<Asset, bool>,
}

impl Global for FirstPage {}

pub(crate) fn begin(owner: WeakEntity<AppView>, window: &Window, cx: &mut App) {
    cx.set_global(FirstPage {
        owner,
        window: window.window_handle().window_id(),
        assets: HashMap::new(),
    });
}

// Only painted viewport entries participate; background library preparation does not.
pub(crate) fn visible(asset: Asset, ready: bool, window: &Window, cx: &mut App) {
    if cx.has_global::<FirstPage>() {
        let page = cx.global_mut::<FirstPage>();
        if page.window == window.window_handle().window_id() {
            page.assets.insert(asset, ready);
        }
    }
}

pub(crate) fn changed(asset: Asset, ready: bool, cx: &mut App) {
    if !cx.has_global::<FirstPage>() {
        return;
    }
    let page = cx.global_mut::<FirstPage>();
    let Some(previous) = page.assets.get_mut(&asset) else {
        return;
    };
    if *previous == ready {
        return;
    }
    *previous = ready;
    let owner = page.owner.clone();
    let _ = owner.update(cx, |_, cx| cx.notify());
}

pub(crate) fn forget(asset: Asset, cx: &mut App) {
    if cx.has_global::<FirstPage>() {
        let page = cx.global_mut::<FirstPage>();
        if page.assets.remove(&asset).is_none() {
            return;
        }
        let owner = page.owner.clone();
        let _ = owner.update(cx, |_, cx| cx.notify());
    }
}

pub(crate) fn ready(cx: &App) -> bool {
    cx.has_global::<FirstPage>() && cx.global::<FirstPage>().assets.values().all(|ready| *ready)
}

pub(crate) fn finish(cx: &mut App) {
    if cx.has_global::<FirstPage>() {
        cx.remove_global::<FirstPage>();
    }
}
