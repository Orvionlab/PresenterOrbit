use serde_json::Value;
use std::collections::HashSet;

pub(crate) fn validate_orders(workspace: &Value) -> Result<(), String> {
    let Some(collections) = workspace.get("macroCollections") else {
        return Ok(());
    };
    let collections = collections
        .as_array()
        .ok_or("Macro collections must be an array.")?;
    let macros = workspace["macros"]
        .as_array()
        .ok_or("Macros must be an array.")?;
    for collection in collections {
        let Some(order) = collection.get("macroOrder") else {
            continue;
        };
        let order = order
            .as_array()
            .ok_or("Macro collection order must be an array.")?;
        let id = collection["id"]
            .as_str()
            .ok_or("Macro collection identity is required.")?;
        let members = macros
            .iter()
            .filter(|macro_value| {
                macro_value["collectionIds"]
                    .as_array()
                    .is_some_and(|ids| ids.iter().any(|value| value.as_str() == Some(id)))
            })
            .map(|value| value["id"].as_str().ok_or("Macro identity is required."))
            .collect::<Result<HashSet<_>, _>>()?;
        let ids = order
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .ok_or("Macro collection order identities must be strings.")
            })
            .collect::<Result<HashSet<_>, _>>()?;
        if ids.len() != order.len() || ids != members {
            return Err("Macro collection order must contain each member exactly once.".into());
        }
    }
    Ok(())
}
