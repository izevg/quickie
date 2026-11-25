//! JSON Schema definitions for the isolated `Profile::Agent` MCP tools.

use serde_json::{Value, json};

use super::super::wire::MAX_SELECTOR_BYTES;
use super::types::{
    DEFAULT_ACTION_TIMEOUT_MS, DEFAULT_LOCATE_RESULTS, DEFAULT_NAVIGATE_TIMEOUT_MS,
    DEFAULT_SCREENSHOT_MAX_BYTES, DEFAULT_VIEWPORT_HEIGHT, DEFAULT_VIEWPORT_WIDTH,
    MAX_ACTION_TIMEOUT_MS, MAX_ELEMENT_ID_BYTES, MAX_INPUT_TEXT_BYTES, MAX_KEY_MODIFIERS,
    MAX_LOCATE_RESULTS, MAX_LOCATOR_TEXT_BYTES, MAX_ROLE_BYTES, MAX_SCREENSHOT_BYTES,
    MAX_SCROLL_DELTA, MAX_URL_BYTES, MAX_VIEWPORT_HEIGHT, MAX_VIEWPORT_WIDTH,
    MIN_ACTION_TIMEOUT_MS, MIN_VIEWPORT_HEIGHT, MIN_VIEWPORT_WIDTH,
};

fn context_id_schema() -> Value {
    json!({
        "type": "integer",
        "minimum": 0
    })
}

fn element_ref_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["context_id", "epoch", "element_id"],
        "properties": {
            "context_id": context_id_schema(),
            "epoch": { "type": "integer", "minimum": 0 },
            "element_id": {
                "type": "string",
                "minLength": 1,
                "maxLength": MAX_ELEMENT_ID_BYTES
            }
        }
    })
}

fn locator_schema() -> Value {
    json!({
        "oneOf": [
            {
                "type": "object",
                "additionalProperties": false,
                "required": ["kind", "selector"],
                "properties": {
                    "kind": { "const": "css" },
                    "selector": {
                        "type": "string",
                        "minLength": 1,
                        "maxLength": MAX_SELECTOR_BYTES
                    }
                }
            },
            {
                "type": "object",
                "additionalProperties": false,
                "required": ["kind", "text"],
                "properties": {
                    "kind": { "const": "text" },
                    "text": {
                        "type": "string",
                        "minLength": 1,
                        "maxLength": MAX_LOCATOR_TEXT_BYTES
                    },
                    "exact": { "type": "boolean", "default": false }
                }
            },
            {
                "type": "object",
                "additionalProperties": false,
                "required": ["kind", "role"],
                "properties": {
                    "kind": { "const": "role" },
                    "role": {
                        "type": "string",
                        "minLength": 1,
                        "maxLength": MAX_ROLE_BYTES
                    },
                    "name": {
                        "type": "string",
                        "minLength": 1,
                        "maxLength": MAX_LOCATOR_TEXT_BYTES
                    },
                    "exact": { "type": "boolean", "default": false }
                }
            }
        ]
    })
}

fn modifiers_schema() -> Value {
    json!({
        "type": "array",
        "maxItems": MAX_KEY_MODIFIERS,
        "uniqueItems": true,
        "items": {
            "type": "string",
            "enum": ["shift", "control", "alt", "meta"]
        }
    })
}

fn mouse_button_schema() -> Value {
    json!({
        "type": "string",
        "enum": ["left", "middle", "right"],
        "default": "left"
    })
}

/// JSON Schema for `agent_context_open`.
#[must_use]
pub fn schema_agent_context_open() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "viewport": {
                "type": "object",
                "additionalProperties": false,
                "required": ["width", "height"],
                "properties": {
                    "width": {
                        "type": "integer",
                        "minimum": MIN_VIEWPORT_WIDTH,
                        "maximum": MAX_VIEWPORT_WIDTH,
                        "default": DEFAULT_VIEWPORT_WIDTH
                    },
                    "height": {
                        "type": "integer",
                        "minimum": MIN_VIEWPORT_HEIGHT,
                        "maximum": MAX_VIEWPORT_HEIGHT,
                        "default": DEFAULT_VIEWPORT_HEIGHT
                    }
                }
            }
        }
    })
}

/// JSON Schema for `agent_context_list`.
#[must_use]
pub fn schema_agent_context_list() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {}
    })
}

/// JSON Schema for `agent_context_close`.
#[must_use]
pub fn schema_agent_context_close() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["context_id"],
        "properties": {
            "context_id": context_id_schema()
        }
    })
}

/// JSON Schema for `agent_navigate`.
#[must_use]
pub fn schema_agent_navigate() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["context_id", "url"],
        "properties": {
            "context_id": context_id_schema(),
            "url": {
                "type": "string",
                "minLength": 1,
                "maxLength": MAX_URL_BYTES
            },
            "wait_stage": {
                "type": "string",
                "enum": ["none", "dom_content_loaded", "load"],
                "default": "load"
            },
            "timeout_ms": {
                "type": "integer",
                "minimum": MIN_ACTION_TIMEOUT_MS,
                "maximum": MAX_ACTION_TIMEOUT_MS,
                "default": DEFAULT_NAVIGATE_TIMEOUT_MS
            }
        }
    })
}

/// JSON Schema for `agent_locate`.
#[must_use]
pub fn schema_agent_locate() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["context_id", "locator"],
        "properties": {
            "context_id": context_id_schema(),
            "locator": locator_schema(),
            "max_results": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_LOCATE_RESULTS,
                "default": DEFAULT_LOCATE_RESULTS
            }
        }
    })
}

fn pointer_action_schema(kind: &str) -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["kind", "element"],
        "properties": {
            "kind": { "const": kind },
            "element": element_ref_schema(),
            "button": mouse_button_schema(),
            "modifiers": modifiers_schema()
        }
    })
}

fn keyboard_action_schemas() -> [Value; 3] {
    [
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["kind", "element", "text"],
            "properties": {
                "kind": { "const": "fill" },
                "element": element_ref_schema(),
                "text": {
                    "type": "string",
                    "maxLength": MAX_INPUT_TEXT_BYTES
                }
            }
        }),
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["kind", "text"],
            "properties": {
                "kind": { "const": "type" },
                "element": element_ref_schema(),
                "text": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": MAX_INPUT_TEXT_BYTES
                }
            }
        }),
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["kind", "key"],
            "properties": {
                "kind": { "const": "press_key" },
                "element": element_ref_schema(),
                "key": { "type": "string", "minLength": 1, "maxLength": 16 },
                "modifiers": modifiers_schema()
            }
        }),
    ]
}

/// JSON Schema for `agent_act`.
#[must_use]
pub fn schema_agent_act() -> Value {
    let [fill_schema, type_schema, press_key_schema] = keyboard_action_schemas();
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["context_id", "action"],
        "properties": {
            "context_id": context_id_schema(),
            "action": {
                "oneOf": [
                    pointer_action_schema("click"),
                    pointer_action_schema("double_click"),
                    {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["kind", "element"],
                        "properties": {
                            "kind": { "const": "hover" },
                            "element": element_ref_schema()
                        }
                    },
                    fill_schema,
                    type_schema,
                    press_key_schema,
                    {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["kind", "delta_x", "delta_y"],
                        "properties": {
                            "kind": { "const": "scroll" },
                            "element": element_ref_schema(),
                            "delta_x": {
                                "type": "integer",
                                "minimum": -MAX_SCROLL_DELTA,
                                "maximum": MAX_SCROLL_DELTA
                            },
                            "delta_y": {
                                "type": "integer",
                                "minimum": -MAX_SCROLL_DELTA,
                                "maximum": MAX_SCROLL_DELTA
                            }
                        }
                    },
                    {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["kind", "element"],
                        "properties": {
                            "kind": { "const": "select_option" },
                            "element": element_ref_schema(),
                            "value": {
                                "type": "string",
                                "minLength": 1,
                                "maxLength": MAX_INPUT_TEXT_BYTES
                            },
                            "label": {
                                "type": "string",
                                "minLength": 1,
                                "maxLength": MAX_INPUT_TEXT_BYTES
                            }
                        }
                    }
                ]
            }
        }
    })
}

/// JSON Schema for `agent_wait`.
#[must_use]
pub fn schema_agent_wait() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["context_id", "condition"],
        "properties": {
            "context_id": context_id_schema(),
            "condition": {
                "oneOf": [
                    {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["kind", "locator"],
                        "properties": {
                            "kind": { "const": "locator_visible" },
                            "locator": locator_schema()
                        }
                    },
                    {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["kind", "locator"],
                        "properties": {
                            "kind": { "const": "locator_hidden" },
                            "locator": locator_schema()
                        }
                    },
                    {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["kind", "text"],
                        "properties": {
                            "kind": { "const": "text_present" },
                            "text": {
                                "type": "string",
                                "minLength": 1,
                                "maxLength": MAX_LOCATOR_TEXT_BYTES
                            },
                            "exact": { "type": "boolean", "default": false }
                        }
                    },
                    {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["kind", "stage"],
                        "properties": {
                            "kind": { "const": "load_stage" },
                            "stage": {
                                "type": "string",
                                "enum": ["dom_content_loaded", "load"]
                            }
                        }
                    }
                ]
            },
            "timeout_ms": {
                "type": "integer",
                "minimum": MIN_ACTION_TIMEOUT_MS,
                "maximum": MAX_ACTION_TIMEOUT_MS,
                "default": DEFAULT_ACTION_TIMEOUT_MS
            }
        }
    })
}

/// JSON Schema for `agent_screenshot`.
#[must_use]
pub fn schema_agent_screenshot() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["context_id"],
        "properties": {
            "context_id": context_id_schema(),
            "target": {
                "oneOf": [
                    {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["kind"],
                        "properties": {
                            "kind": { "const": "viewport" }
                        }
                    },
                    {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["kind", "element"],
                        "properties": {
                            "kind": { "const": "element" },
                            "element": element_ref_schema()
                        }
                    }
                ]
            },
            "max_bytes": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_SCREENSHOT_BYTES,
                "default": DEFAULT_SCREENSHOT_MAX_BYTES
            }
        }
    })
}
