//! Compiles a narrow schema for an input action tree from the reflection database.
//!
//! One fact is written here by hand: which classes the schema covers and how
//! they nest. Roblox publishes it nowhere, because it is not a property of a
//! class. Everything else is read out of the bundled reflection database, so
//! the schema follows Roblox rather than a snapshot of it: a property added to
//! `InputBinding` appears in the schema, a property that turns read-only
//! leaves it, and the members of every enum are whatever the database says
//! they are.
//!
//! Which binding properties suit the `Type` of the action above them is left
//! alone on purpose. Roblox documents none of it, and a rule written from a
//! guess would reject files that are perfectly good.

use std::collections::BTreeSet;

use anyhow::{Context, Result};
use rbx_reflection::{
    ClassDescriptor, DataType, PropertyDescriptor, PropertyKind, PropertySerialization,
    PropertyTag, ReflectionDatabase, Scriptability,
};
use rbx_types::VariantType;
use serde_json::{json, Map, Value};

/// The classes the schema covers, each with the single class its children may
/// be. `InputBinding` is a leaf, so it is given no children key at all.
const HIERARCHY: &[(&str, Option<&str>)] = &[
    ("InputContext", Some("InputAction")),
    ("InputAction", Some("InputBinding")),
    ("InputBinding", None),
];

/// The class a file has to start with. A file holds one context.
const ROOT: &str = "InputContext";

/// The name of the definition holding Rojo's explicit property value form.
const FULLY_QUALIFIED: &str = "FullyQualifiedValue";
/// The name of the definition holding an attribute value.
const ATTRIBUTE: &str = "AttributeValue";

/// A compiled schema body, its definitions, and where they came from.
pub struct Compiled {
    pub body: Value,
    pub defs: Map<String, Value>,
    /// The reflection database release, as `0.728.0.7280895`.
    pub version: String,
}

/// Compiles the input action tree schema.
pub fn input_action_system() -> Result<Compiled> {
    let database = rbx_reflection_database::get_bundled();
    let mut defs = Map::new();
    let mut enums = BTreeSet::new();

    for (class, child) in HIERARCHY {
        let descriptor = database
            .classes
            .get(class)
            .with_context(|| format!("{class} is missing from the reflection database"))?;

        defs.insert((*class).to_owned(), node_def(descriptor, *child));
        defs.insert(
            properties_def_name(class),
            properties_def(database, descriptor, &mut enums),
        );
    }

    for name in enums {
        let def = enum_def(database, &name)?;
        defs.insert(name, def);
    }

    defs.insert(FULLY_QUALIFIED.to_owned(), fully_qualified_def());
    defs.insert(ATTRIBUTE.to_owned(), attribute_def());

    // A file holds exactly one context, with its actions and their bindings
    // nested inside it.
    let body = reference(ROOT);

    Ok(Compiled {
        body,
        defs,
        version: version(database),
    })
}

/// The node itself: the keys Rojo reads off a JSON model, narrowed to one class.
fn node_def(descriptor: &ClassDescriptor, child: Option<&str>) -> Value {
    let class = descriptor.name;
    let mut keys = Map::new();

    keys.insert(
        "$schema".into(),
        json!({
            "type": ["string", "null"],
            "description": "The schema this file is written against.",
        }),
    );

    keys.insert("className".into(), json!({ "const": class }));
    keys.insert(
        "ClassName".into(),
        json!({ "const": class, "description": "Alias of `className`." }),
    );

    // Rojo takes the root instance name from the file name and only warns about
    // a name written on the root, so the key is allowed rather than required.
    keys.insert(
        "name".into(),
        json!({
            "type": ["string", "null"],
            "description": "The instance name. Rojo ignores it on the root of a `.model.json`, where the file name wins, and uses it on every child.",
        }),
    );
    keys.insert(
        "Name".into(),
        json!({ "type": ["string", "null"], "description": "Alias of `name`." }),
    );

    keys.insert(
        "id".into(),
        json!({
            "type": ["string", "null"],
            "description": "An identifier other nodes can point a `Ref` property at.",
        }),
    );

    keys.insert(
        "attributes".into(),
        json!({
            "type": "object",
            "additionalProperties": reference(ATTRIBUTE),
            "description": "Attributes set on the instance.",
        }),
    );

    let properties = properties_def_name(class);
    keys.insert(
        "properties".into(),
        json!({
            "$ref": format!("#/$defs/{properties}"),
            "description": format!("Properties set on the `{class}`."),
        }),
    );
    keys.insert(
        "Properties".into(),
        json!({
            "$ref": format!("#/$defs/{properties}"),
            "description": "Alias of `properties`.",
        }),
    );

    if let Some(child) = child {
        keys.insert(
            "children".into(),
            json!({
                "type": "array",
                "items": reference(child),
                "description": format!("The `{child}` instances under this `{class}`."),
            }),
        );
        keys.insert(
            "Children".into(),
            json!({
                "type": "array",
                "items": reference(child),
                "description": "Alias of `children`.",
            }),
        );
    }

    let description = match child {
        Some(child) => format!("A `{class}` node, holding `{child}` instances."),
        None => format!("A `{class}` node. Nothing nests under it."),
    };

    json!({
        "type": "object",
        "title": class,
        "description": description,
        "properties": Value::Object(keys),
        // Deliberately stricter than Rojo, which ignores keys it does not know.
        // A file opts into this schema to have its typos caught.
        "additionalProperties": false,
        // Rojo needs the class name, and serde refuses a key given twice under
        // both of its spellings.
        "oneOf": [
            { "required": ["className"] },
            { "required": ["ClassName"] },
        ],
    })
}

/// The properties the class exposes, each typed the way Rojo resolves it.
fn properties_def(
    database: &ReflectionDatabase,
    descriptor: &ClassDescriptor,
    enums: &mut BTreeSet<String>,
) -> Value {
    let mut fields = Map::new();

    // Walking up to `Instance` keeps the inherited properties a file may set,
    // `Name` and `Archivable` among them. The class comes first, so a property
    // it redefines wins over the one it inherits.
    for class in database.superclasses_iter(descriptor) {
        for (name, property) in &class.properties {
            if !is_settable(property) || fields.contains_key(*name) {
                continue;
            }

            fields.insert((*name).to_string(), field(class.name, property, enums));
        }
    }

    json!({
        "type": "object",
        "title": format!("{} properties", descriptor.name),
        "description": format!(
            "Properties of a `{}`. Read-only and non-serialising members are left out: \
             a file cannot set them.",
            descriptor.name
        ),
        "properties": Value::Object(fields),
        "additionalProperties": false,
    })
}

/// Whether a `.model.json` can carry the property at all.
///
/// Rojo hands the value to the reflection database, so a property that does not
/// serialise, that Roblox marks read-only, or that scripts cannot write is one
/// no file can usefully set. The state members of `InputAction` are the case
/// that matters here: `BoolState` and the direction states are outputs of the
/// engine, not inputs to it.
fn is_settable(property: &PropertyDescriptor) -> bool {
    let serialises = matches!(
        property.kind,
        PropertyKind::Canonical {
            serialization: PropertySerialization::Serializes
                | PropertySerialization::SerializesAs(_)
        }
    );

    let writable = matches!(
        property.scriptability,
        Scriptability::ReadWrite | Scriptability::Write
    );

    let concealed = property.tags.iter().any(|tag| {
        matches!(
            tag,
            PropertyTag::ReadOnly
                | PropertyTag::Deprecated
                | PropertyTag::Hidden
                | PropertyTag::NotScriptable
        )
    });

    serialises && writable && !concealed
}

/// One property: its shorthand form, or the explicit form when it has none.
fn field(class: &str, property: &PropertyDescriptor, enums: &mut BTreeSet<String>) -> Value {
    let name = property.name;

    match shorthand(&property.data_type, enums) {
        Some(shorthand) => json!({
            "description": format!("`{class}.{name}`, a {} property.", type_name(&property.data_type)),
            "anyOf": [shorthand, reference(FULLY_QUALIFIED)],
        }),
        // A `Ref` is the case in practice: Rojo refuses to read one from a
        // shorthand and points at an attribute instead.
        None => json!({
            "description": format!(
                "`{class}.{name}`, a {} property. It has no shorthand form, so it has to be \
                 written out in full, and Rojo may want it as a ref pointer attribute instead.",
                type_name(&property.data_type)
            ),
            "$ref": format!("#/$defs/{FULLY_QUALIFIED}"),
        }),
    }
}

/// The JSON shape Rojo resolves for the type, or `None` when it resolves none.
///
/// This mirrors `AmbiguousValue::resolve` in the vendored `resolution.rs`. A
/// type missing from that match has no shorthand, and saying so is better than
/// accepting a value Rojo will reject.
fn shorthand(data_type: &DataType, enums: &mut BTreeSet<String>) -> Option<Value> {
    match data_type {
        DataType::Enum(name) => {
            enums.insert((*name).to_owned());
            Some(reference(name))
        }
        DataType::Value(variant) => value_shorthand(*variant),
        _ => None,
    }
}

fn value_shorthand(variant: VariantType) -> Option<Value> {
    let numbers = |count| {
        json!({
            "type": "array",
            "items": { "type": "number" },
            "minItems": count,
            "maxItems": count,
        })
    };

    Some(match variant {
        VariantType::Bool => json!({ "type": "boolean" }),
        VariantType::Float32 | VariantType::Float64 | VariantType::Int32 | VariantType::Int64 => {
            json!({ "type": "number" })
        }
        VariantType::String | VariantType::Content | VariantType::ContentId => {
            json!({ "type": "string" })
        }
        VariantType::Tags => json!({ "type": "array", "items": { "type": "string" } }),
        VariantType::Vector2 => numbers(2),
        VariantType::Vector3 | VariantType::Color3 => numbers(3),
        VariantType::CFrame => numbers(12),
        VariantType::Attributes | VariantType::Font | VariantType::MaterialColors => {
            json!({ "type": "object" })
        }
        _ => return None,
    })
}

/// The Roblox name of the type, for prose only.
fn type_name(data_type: &DataType) -> String {
    match data_type {
        DataType::Enum(name) => format!("`Enum.{name}`"),
        // `VariantType` renders as the Roblox type name, `Vector2` and so on.
        DataType::Value(variant) => format!("`{variant:?}`"),
        _ => "an unrecognised".to_owned(),
    }
}

fn enum_def(database: &ReflectionDatabase, name: &str) -> Result<Value> {
    let descriptor = database
        .enums
        .get(name)
        .with_context(|| format!("enum {name} is missing from the reflection database"))?;

    let mut items: Vec<&str> = descriptor.items.keys().copied().collect();
    items.sort_unstable();

    Ok(json!({
        "type": "string",
        "title": format!("Enum.{name}"),
        "description": format!("A member of the Roblox `{name}` enum, written as its name."),
        "enum": items,
    }))
}

fn fully_qualified_def() -> Value {
    json!({
        "type": "object",
        "title": "Fully qualified value",
        "description": "A property value in Rojo's explicit form: an object keyed by the Roblox \
                        type, such as `{ \"Vector3\": [0, 1, 0] }`. Rojo accepts this for any \
                        property, so what is inside cannot be checked here.",
    })
}

fn attribute_def() -> Value {
    json!({
        "title": "Attribute value",
        "description": "An attribute value, either a shorthand Rojo can resolve on its own or a \
                        fully qualified object. An attribute carries no declared type, so anything \
                        JSON can express belongs here.",
    })
}

fn properties_def_name(class: &str) -> String {
    format!("{class}Properties")
}

fn reference(name: &str) -> Value {
    json!({ "$ref": format!("#/$defs/{name}") })
}

fn version(database: &ReflectionDatabase) -> String {
    database
        .version
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(".")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compiled() -> Compiled {
        input_action_system().unwrap()
    }

    #[test]
    fn covers_the_three_classes_and_nothing_else() {
        let defs = compiled().defs;

        for class in ["InputContext", "InputAction", "InputBinding"] {
            assert!(defs.contains_key(class), "lost the {class} definition");
        }
        assert!(!defs.contains_key("InputActionService"));
        assert!(!defs.contains_key("Folder"));
    }

    #[test]
    fn nests_each_class_under_the_one_above_it() {
        let defs = compiled().defs;

        assert_eq!(
            defs["InputContext"]["properties"]["children"]["items"]["$ref"],
            "#/$defs/InputAction"
        );
        assert_eq!(
            defs["InputAction"]["properties"]["children"]["items"]["$ref"],
            "#/$defs/InputBinding"
        );
        // A leaf carries no children key, and unknown keys are refused, so the
        // schema rejects children rather than accepting an empty list of them.
        assert!(defs["InputBinding"]["properties"]["children"].is_null());
        assert_eq!(defs["InputBinding"]["additionalProperties"], false);
    }

    #[test]
    fn reads_each_type_property_from_its_own_enum() {
        let defs = compiled().defs;

        // The two are different enums that happen to share a property name.
        assert_eq!(
            defs["InputActionProperties"]["properties"]["Type"]["anyOf"][0]["$ref"],
            "#/$defs/InputActionType"
        );
        assert_eq!(
            defs["InputBindingProperties"]["properties"]["Type"]["anyOf"][0]["$ref"],
            "#/$defs/InputBindingType"
        );
        assert_eq!(
            defs["InputBindingProperties"]["properties"]["KeyCode"]["anyOf"][0]["$ref"],
            "#/$defs/KeyCode"
        );
    }

    #[test]
    fn carries_only_the_enums_it_references() {
        let defs = compiled().defs;

        for name in ["InputActionType", "InputBindingType", "KeyCode"] {
            assert!(defs.contains_key(name), "lost the {name} enum");
        }
        // The database holds hundreds of enums. Emitting the ones no property
        // mentions would multiply the size of the schema for nothing.
        assert!(!defs.contains_key("Material"));
        assert!(!defs.contains_key("UserInputType"));
    }

    #[test]
    fn drops_the_members_a_file_cannot_set() {
        let defs = compiled().defs;
        let action = &defs["InputActionProperties"]["properties"];

        // Engine outputs, marked read-only and non-serialising.
        for state in [
            "BoolState",
            "Direction1DState",
            "Direction2DState",
            "Direction3DState",
            "ViewportPositionState",
        ] {
            assert!(action[state].is_null(), "{state} should not be settable");
        }

        assert!(action["Enabled"].is_object());
        assert!(action["Type"].is_object());
        // Inherited from Instance, and settable.
        assert!(action["Name"].is_object());
        // Inherited but not settable: Parent does not serialise, and Rojo takes
        // attributes and tags through their own keys.
        for hidden in ["Parent", "Attributes", "Tags"] {
            assert!(action[hidden].is_null(), "{hidden} should not be settable");
        }
    }

    #[test]
    fn types_a_property_the_way_rojo_resolves_it() {
        let defs = compiled().defs;
        let binding = &defs["InputBindingProperties"]["properties"];

        assert_eq!(
            binding["ClampMagnitudeToOne"]["anyOf"][0]["type"],
            "boolean"
        );
        assert_eq!(binding["PressedThreshold"]["anyOf"][0]["type"], "number");
        assert_eq!(binding["Vector2Scale"]["anyOf"][0]["minItems"], 2);
        assert_eq!(binding["Vector3Scale"]["anyOf"][0]["maxItems"], 3);
        // A Ref has no shorthand at all, so only the explicit form is offered.
        assert_eq!(
            binding["UIButton"]["$ref"],
            format!("#/$defs/{FULLY_QUALIFIED}")
        );
        assert!(binding["UIButton"]["anyOf"].is_null());
    }

    #[test]
    fn starts_a_file_at_a_single_context() {
        let compiled = compiled();

        assert_eq!(compiled.body["$ref"], "#/$defs/InputContext");
        // An action or a binding on its own is not a file.
        assert!(compiled.body["anyOf"].is_null());
    }

    #[test]
    fn compiles_the_same_schema_twice() {
        let first = compiled();
        let second = compiled();

        assert_eq!(first.defs, second.defs);
        assert_eq!(first.body, second.body);
        assert_eq!(first.version, second.version);
    }
}
