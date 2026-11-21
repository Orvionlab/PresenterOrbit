use serde_json::{Map, Value};

#[derive(Default)]
pub(crate) struct Drafts {
    next: u64,
    pending: Vec<(u64, Value)>,
    scoped: Vec<Scoped>,
}

struct Scoped {
    request: u64,
    path: String,
    section: String,
    patch: Map<String, Value>,
}

pub(super) fn merge(settings: &mut Value, patch: &Value) {
    for (section, value) in patch.as_object().into_iter().flatten() {
        if let (Some(target), Some(fields)) = (settings[section].as_object_mut(), value.as_object())
        {
            target.extend(fields.clone());
        } else {
            settings[section] = value.clone();
        }
    }
}

impl Drafts {
    pub fn shown(&self, canonical: &Value) -> Value {
        let mut shown = canonical.clone();
        for (_, patch) in &self.pending {
            merge(&mut shown, patch);
        }
        shown
    }

    pub fn begin(&mut self, canonical: &Value, requested: &Value) -> (u64, Value) {
        let before = self.shown(canonical);
        let mut patch = Map::new();
        for (section, value) in requested.as_object().into_iter().flatten() {
            if let Some(fields) = value.as_object() {
                let changed: Map<_, _> = fields
                    .iter()
                    .filter(|(key, value)| before[section].get(*key) != Some(*value))
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect();
                if !changed.is_empty() {
                    patch.insert(section.clone(), Value::Object(changed));
                }
            } else if before.get(section) != Some(value) {
                patch.insert(section.clone(), value.clone());
            }
        }
        self.next += 1;
        let patch = Value::Object(patch);
        self.pending.push((self.next, patch.clone()));
        (self.next, patch)
    }

    pub fn finish(&mut self, request: u64) {
        self.pending.retain(|(id, _)| *id != request);
        self.scoped.retain(|draft| draft.request != request);
    }

    pub fn section_patch(&self, path: &str, section: &str) -> Map<String, Value> {
        self.scoped
            .iter()
            .filter(|draft| draft.path == path && draft.section == section)
            .flat_map(|draft| draft.patch.iter())
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()
    }

    pub fn section(&self, path: &str, section: &str, canonical: &Value) -> Value {
        let mut shown = canonical.clone();
        for draft in self
            .scoped
            .iter()
            .filter(|draft| draft.path == path && draft.section == section)
        {
            for (key, value) in &draft.patch {
                if value.is_null() {
                    if let Some(fields) = shown.as_object_mut() {
                        fields.remove(key);
                    }
                } else {
                    shown[key] = value.clone();
                }
            }
        }
        shown
    }

    pub fn begin_section(
        &mut self,
        path: &str,
        section: &str,
        before: &Value,
        requested: &Value,
    ) -> u64 {
        let mut patch = Map::new();
        for key in before
            .as_object()
            .into_iter()
            .flat_map(|fields| fields.keys())
            .chain(
                requested
                    .as_object()
                    .into_iter()
                    .flat_map(|fields| fields.keys()),
            )
        {
            if before.get(key) != requested.get(key) {
                patch.insert(
                    key.clone(),
                    requested.get(key).cloned().unwrap_or(Value::Null),
                );
            }
        }
        self.begin_section_patch(path, section, patch)
    }

    pub fn begin_section_patch(
        &mut self,
        path: &str,
        section: &str,
        patch: Map<String, Value>,
    ) -> u64 {
        self.next += 1;
        self.scoped.push(Scoped {
            request: self.next,
            path: path.into(),
            section: section.into(),
            patch,
        });
        self.next
    }
}
