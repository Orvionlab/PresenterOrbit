use super::*;
use crate::Host;

const METHODS: [&str; 9] = [
    "listBibles",
    "importBiblePackage",
    "installWebBible",
    "removeBible",
    "getBibleBooks",
    "resolveBibleReference",
    "getBiblePassage",
    "searchBible",
    "createBiblePresentation",
];

pub fn handles(method: &str) -> bool {
    METHODS.contains(&method)
}

pub fn run(app: &Host, state: &AppState, method: &str, args: &[Value]) -> Result<Value, String> {
    let root = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("Bibles");
    let store = bible::Store::open(&root)?;
    match method {
        "listBibles" => encode(store.list()?),
        "importBiblePackage" => {
            let Some(path) = FileDialog::new()
                .set_title("Install Bible translation package")
                .add_filter("Orbit Bible package", &["json"])
                .pick_file()
            else {
                return Ok(json!({"canceled":true}));
            };
            Ok(json!({"canceled":false,"translation":store.install(&path)?}))
        }
        "installWebBible" => encode(store.install_web()?),
        "removeBible" => {
            store.remove(&text(args, 0)?)?;
            Ok(Value::Null)
        }
        "getBibleBooks" => encode(store.books(&text(args, 0)?)?),
        "resolveBibleReference" => {
            encode(store.resolve_reference(&text(args, 0)?, &text(args, 1)?)?)
        }
        "getBiblePassage" => encode(store.passage(&decode(argument(args, 0)?)?)?),
        "searchBible" => encode(store.search(&decode(argument(args, 0)?)?)?),
        "createBiblePresentation" => {
            let path = active_workspace(state, argument(args, 0)?)?;
            let mut io = lock_workspace_io(state)?;
            io.require_revision(&path, expected_workspace_revision(args, 2)?)?;
            let (workspace, item_id, entry_id) =
                bible_presentation::create(&path, &store, &decode(argument(args, 1)?)?)?;
            let mut response = json!({"workspace":workspace,"revision":io.advance_revision(&path),"itemId":item_id});
            if let Some(id) = entry_id {
                response["playlistEntryId"] = json!(id);
            }
            Ok(response)
        }
        _ => Err("Unknown Bible operation.".into()),
    }
}

fn text(args: &[Value], index: usize) -> Result<String, String> {
    argument(args, index)?
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| "A nonempty Bible identifier or query is required.".into())
}

fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|error| format!("Invalid Bible request: {error}"))
}

fn encode(value: impl serde::Serialize) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|error| error.to_string())
}
