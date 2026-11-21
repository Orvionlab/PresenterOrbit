use super::document::CanvasDocument;
use futures::{future::Shared, FutureExt};
use gpui::{App, Task};
use serde_json::Value;
use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::Arc};

type Saved = Result<Rc<dyn CanvasDocument>, String>;
type Pending = Shared<Task<Saved>>;

struct State {
    document: Rc<dyn CanvasDocument>,
    slide: Arc<Value>,
    pending: Option<Pending>,
    request: u64,
    failed: bool,
}

#[derive(Default)]
pub(super) struct Documents(HashMap<String, Rc<RefCell<State>>>);

impl Documents {
    pub fn keep_draft(&mut self, key: &str, slide: Arc<Value>) {
        if let Some(state) = self.0.get(key) {
            let mut state = state.borrow_mut();
            if state.failed {
                state.slide = slide;
            }
        }
    }

    pub fn open(&mut self, document: Rc<dyn CanvasDocument>) -> Rc<dyn CanvasDocument> {
        self.0.retain(|_, state| {
            let state = state.borrow();
            state.pending.is_some() || state.failed
        });
        let key = document.key();
        let state = self.0.entry(key.clone()).or_insert_with(|| {
            Rc::new(RefCell::new(State {
                slide: document.slide(),
                document,
                pending: None,
                request: 0,
                failed: false,
            }))
        });
        Rc::new(Document {
            key,
            state: state.clone(),
        })
    }
}

struct Document {
    key: String,
    state: Rc<RefCell<State>>,
}

impl CanvasDocument for Document {
    fn key(&self) -> String {
        self.key.clone()
    }
    fn slide(&self) -> Arc<Value> {
        self.state.borrow().slide.clone()
    }

    fn commit(&self, slide: Value, cx: &mut App) -> Task<Saved> {
        let state = self.state.clone();
        let (previous, document, request) = {
            let mut state = state.borrow_mut();
            state.request += 1;
            state.slide = Arc::new(slide.clone());
            (state.pending.clone(), state.document.clone(), state.request)
        };
        let pending = cx
            .spawn(async move |cx| {
                let result = match previous {
                    Some(previous) => previous.await,
                    None => Ok(document),
                };
                let result = match result {
                    Ok(document) => cx.update(|cx| document.commit(slide, cx)).await,
                    Err(error) => Err(error),
                };
                let mut state = state.borrow_mut();
                if let Ok(document) = &result {
                    state.document = document.clone();
                }
                if request == state.request {
                    state.pending = None;
                    state.failed = result.is_err();
                    if let Ok(document) = &result {
                        state.slide = document.slide();
                    }
                }
                result
            })
            .shared();
        self.state.borrow_mut().pending = Some(pending.clone());
        cx.spawn(async move |_| pending.await)
    }
}
