#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};
    use serde_json::value::RawValue;
    use serde_json::{Map, Value};
    use std::collections::BTreeMap;

    #[derive(Debug, Serialize, Deserialize)]
    struct Probe {
        id: Box<RawValue>,
        method: String,
        params: Map<String, Value>,
    }

    #[test]
    fn probe_raw_value_paths() {
        for text in [
            "{\"id\":1e400,\"method\":\"echo\",\"params\":{}}",
            "{\"id\":123456789012345678901234567890,\"method\":\"echo\",\"params\":{}}",
            "{\"id\":1e2,\"method\":\"echo\",\"params\":{}}",
            "{\"id\":18446744073709551615,\"method\":\"echo\",\"params\":{}}",
        ] {
            let parsed = serde_json::from_str::<Probe>(text).expect("from_str works");
            println!("FROM_STR id={}", parsed.id.get());
            println!("  REENCODED = {}", serde_json::to_string(&parsed).expect("ser"));
            println!("  TO_VALUE  = {:?}", serde_json::to_value(&parsed).map(|v| v.to_string()));
        }

        println!(
            "RAW_LINE_BAD = {:?}",
            serde_json::from_str::<Box<RawValue>>("this is not json").is_err()
        );
        println!(
            "RAW_MAP_ARRAY_ERR = {:?}",
            serde_json::from_str::<BTreeMap<String, Box<RawValue>>>("[1,\"echo\",{}]").err().map(|e| e.to_string())
        );
        println!(
            "RAW_MAP_OK = {:?}",
            serde_json::from_str::<BTreeMap<String, Box<RawValue>>>("{\"id\":1e400}")
                .map(|m| m.get("id").map(|v| v.get().to_string()))
        );
        println!("NULL_TOKEN = {:?}", RawValue::from_string("null".to_string()).map(|v| v.get().to_string()));
        println!(
            "ARRAY_AS_REQUEST = {:?}",
            serde_json::from_str::<Probe>("[1,\"echo\",{}]").map(|p| p.id.get().to_string())
        );
    }
}
