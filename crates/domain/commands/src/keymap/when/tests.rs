use super::{ContextFlag, FocusedView, MAX_DEPTH, WhenContext, WhenError, WhenExpr};

fn when(text: &str) -> WhenExpr {
    WhenExpr::try_from(text).unwrap()
}

fn error(text: &str) -> WhenError {
    WhenExpr::try_from(text).unwrap_err()
}

fn holds(text: &str, context: &WhenContext) -> bool {
    when(text).evaluate(context)
}

fn flags(keys: &[&str]) -> WhenContext {
    keys.iter()
        .fold(WhenContext::new(), |context, &key| context.with_flag(key))
}

// ---- evaluation --------------------------------------------------------------------------------

#[test]
fn a_set_flag_holds() {
    let context = flags(&["pageFocused"]);

    assert!(holds("pageFocused", &context));
}

#[test]
fn an_unset_flag_does_not_hold() {
    let context = flags(&["pageFocused"]);

    assert!(!holds("inputFocused", &context));
}

#[test]
fn a_key_with_a_value_is_true() {
    let context = WhenContext::new().with_value("focusedView", "palette");

    assert!(holds("focusedView", &context));
}

#[test]
fn a_key_with_an_empty_value_is_false() {
    let context = WhenContext::new().with_value("focusedView", "");

    assert!(!holds("focusedView", &context));
}

#[test]
fn the_literals_are_constant() {
    let context = WhenContext::new();

    assert!(holds("true", &context));
    assert!(!holds("false", &context));
}

#[test]
fn the_literals_ignore_keys_of_the_same_name() {
    let context = WhenContext::new().with_flag("false").with_value("true", "");

    assert!(!holds("false", &context));
    assert!(holds("true", &context));
}

#[test]
fn not_inverts() {
    let context = flags(&["inputFocused"]);

    assert!(!holds("!inputFocused", &context));
    assert!(holds("!pageFocused", &context));
}

#[test]
fn a_double_not_cancels_out() {
    let context = flags(&["a"]);

    assert!(holds("!!a", &context));
}

#[test]
fn and_needs_every_operand() {
    let both = flags(&["a", "b"]);
    let one = flags(&["a"]);

    assert!(holds("a && b", &both));
    assert!(!holds("a && b", &one));
}

#[test]
fn or_needs_one_operand() {
    let one = flags(&["b"]);
    let none = flags(&[]);

    assert!(holds("a || b", &one));
    assert!(!holds("a || b", &none));
}

#[test]
fn equals_compares_the_value() {
    let context = WhenContext::new().with_value("focusedView", "palette");

    assert!(holds("focusedView == 'palette'", &context));
    assert!(!holds("focusedView == 'page'", &context));
}

#[test]
fn equals_is_false_for_a_missing_key() {
    let context = WhenContext::new();

    assert!(!holds("focusedView == 'palette'", &context));
}

#[test]
fn not_equals_compares_the_value() {
    let context = WhenContext::new().with_value("focusedView", "palette");

    assert!(!holds("focusedView != 'palette'", &context));
    assert!(holds("focusedView != 'page'", &context));
}

#[test]
fn not_equals_is_true_for_a_missing_key() {
    let context = WhenContext::new();

    assert!(holds("focusedView != 'palette'", &context));
}

#[test]
fn and_binds_tighter_than_or() {
    let context = flags(&["a"]);

    assert!(holds("a || b && c", &context));
    assert!(!holds("(a || b) && c", &context));
}

#[test]
fn not_binds_tighter_than_and() {
    let context = flags(&["b"]);

    assert!(holds("!a && b", &context));
    assert!(!holds("!(a && b)", &flags(&["a", "b"])));
}

#[test]
fn not_applies_to_a_whole_comparison() {
    let context = WhenContext::new().with_value("view", "x");

    assert!(!holds("!view == 'x'", &context));
}

// ---- Quickie's own context keys ----------------------------------------------------------------

#[test]
fn every_flag_is_found_by_its_name() {
    for flag in ContextFlag::ALL {
        assert_eq!(ContextFlag::from_name(flag.name()), Some(flag));
    }
    assert_eq!(ContextFlag::from_name("editorFocus"), None);
}

#[test]
fn flags_have_the_names_clauses_use() {
    let names = ContextFlag::ALL.map(ContextFlag::name);

    assert_eq!(
        names,
        [
            "pageFocused",
            "inputFocused",
            "paletteOpen",
            "addressOpen",
            "overviewOpen",
            "hintsOpen",
            "switcherOpen",
            "popoverOpen",
            "agentOpen",
        ]
    );
}

#[test]
fn every_view_is_found_by_its_name() {
    for view in FocusedView::ALL {
        assert_eq!(FocusedView::from_name(view.name()), Some(view));
    }
    assert_eq!(FocusedView::from_name("sidebar"), None);
}

#[test]
fn views_have_the_names_clauses_use() {
    let names = FocusedView::ALL.map(FocusedView::name);

    assert_eq!(
        names,
        ["page", "palette", "address", "overview", "settings"]
    );
}

#[test]
fn each_flag_is_set_on_its_own() {
    for flag in ContextFlag::ALL {
        let context = WhenContext::new().with(flag);

        for other in ContextFlag::ALL {
            assert_eq!(holds(other.name(), &context), other == flag, "{other:?}");
        }
    }
}

#[test]
fn a_flag_set_by_name_is_the_flag_set_by_type() {
    for flag in ContextFlag::ALL {
        assert_eq!(
            WhenContext::new().with_flag(flag.name()),
            WhenContext::new().with(flag)
        );
    }
}

#[test]
fn a_view_set_by_name_is_the_view_set_by_type() {
    for view in FocusedView::ALL {
        assert_eq!(
            WhenContext::new().with_value("focusedView", view.name()),
            WhenContext::new().in_view(view)
        );
    }
}

#[test]
fn a_view_compares_by_its_name() {
    let context = WhenContext::new().in_view(FocusedView::Settings);

    assert!(holds("focusedView == 'settings'", &context));
    assert!(!holds("focusedView == 'page'", &context));
    assert!(holds("focusedView != 'page'", &context));
    assert!(holds("focusedView", &context));
}

#[test]
fn a_view_quickie_does_not_have_is_still_a_value() {
    let context = WhenContext::new().with_value("focusedView", "sidebar");

    assert!(holds("focusedView == 'sidebar'", &context));
    assert!(!holds("focusedView == 'page'", &context));
    assert!(holds("focusedView", &context));
}

#[test]
fn a_later_view_replaces_an_earlier_one() {
    let unknown_then_known = WhenContext::new()
        .with_value("focusedView", "sidebar")
        .in_view(FocusedView::Page);
    let known_then_unknown = WhenContext::new()
        .in_view(FocusedView::Page)
        .with_value("focusedView", "sidebar");

    assert!(holds("focusedView == 'page'", &unknown_then_known));
    assert!(!holds("focusedView == 'sidebar'", &unknown_then_known));
    assert!(holds("focusedView == 'sidebar'", &known_then_unknown));
    assert!(!holds("focusedView == 'page'", &known_then_unknown));
}

#[test]
fn setting_the_view_keeps_the_other_values() {
    let context = WhenContext::new()
        .with_value("mode", "insert")
        .in_view(FocusedView::Page);

    assert!(holds("mode == 'insert'", &context));
}

#[test]
fn a_later_value_replaces_an_earlier_one() {
    let context = WhenContext::new()
        .with_value("mode", "insert")
        .with_value("mode", "normal");

    assert!(holds("mode == 'normal'", &context));
    assert!(!holds("mode == 'insert'", &context));
}

#[test]
fn setting_a_flag_twice_is_setting_it_once() {
    let twice = WhenContext::new().with_flag("a").with_flag("a");

    assert_eq!(twice, WhenContext::new().with_flag("a"));
}

#[test]
fn setting_one_of_quickies_flags_twice_keeps_it_set() {
    let twice = WhenContext::new()
        .with(ContextFlag::HintsOpen)
        .with(ContextFlag::HintsOpen);

    assert!(holds("hintsOpen", &twice));
}

#[test]
fn a_view_prints_as_its_name() {
    let clause = when("focusedView == 'palette' || focusedView != 'page'");

    assert_eq!(
        clause.to_string(),
        "focusedView == 'palette' || focusedView != 'page'"
    );
}

#[test]
fn focused_view_set_as_a_flag_is_true() {
    let context = WhenContext::new().with_flag("focusedView");

    assert!(holds("focusedView", &context));
    assert!(!holds("focusedView == 'page'", &context));
}

#[test]
fn one_of_quickies_flags_may_hold_a_value() {
    let context = WhenContext::new().with_value("pageFocused", "yes");

    assert!(holds("pageFocused", &context));
    assert!(holds("pageFocused == 'yes'", &context));
    assert!(!holds(
        "pageFocused",
        &WhenContext::new().with_value("pageFocused", "")
    ));
}

#[test]
fn a_view_name_compared_with_another_key_is_plain_text() {
    let context = WhenContext::new().with_value("panel", "page");

    assert!(holds("panel == 'page'", &context));
    assert!(!holds("focusedView == 'page'", &context));
}

// ---- canonical text ----------------------------------------------------------------------------

#[test]
fn spacing_does_not_change_a_clause() {
    let tight = when("a&&!b||c");
    let loose = when("  a  &&  ! b ||\tc ");

    assert_eq!(tight, loose);
    assert_eq!(tight.to_string(), "a && !b || c");
}

#[test]
fn redundant_parentheses_are_dropped() {
    let clause = when("((a)) && (!b) || (c == 'x')");

    assert_eq!(clause.to_string(), "a && !b || c == 'x'");
}

#[test]
fn a_negated_group_keeps_its_parentheses() {
    assert_eq!(when("!(a && b)").to_string(), "!(a && b)");
    assert_eq!(when("!(a || b)").to_string(), "!(a || b)");
}

#[test]
fn a_negated_single_operand_needs_no_parentheses() {
    let clause = when("!(a)");

    assert_eq!(clause.to_string(), "!a");
}

#[test]
fn an_or_inside_an_and_keeps_its_parentheses() {
    let clause = when("(a || b) && c");

    assert_eq!(clause.to_string(), "(a || b) && c");
}

#[test]
fn a_grouped_and_inside_an_and_keeps_its_parentheses() {
    let clause = when("a && (b && c)");

    assert_eq!(clause.to_string(), "a && (b && c)");
}

#[test]
fn a_grouped_or_inside_an_or_keeps_its_parentheses() {
    let clause = when("a || (b || c)");

    assert_eq!(clause.to_string(), "a || (b || c)");
}

#[test]
fn an_and_inside_an_or_needs_no_parentheses() {
    let clause = when("(a && b) || c");

    assert_eq!(clause.to_string(), "a && b || c");
}

#[test]
fn a_comparison_inside_an_and_needs_no_parentheses() {
    let clause = when("view != 'x' && a");

    assert_eq!(clause.to_string(), "view != 'x' && a");
}

#[test]
fn double_quotes_become_single_quotes() {
    let clause = when("view == \"palette\"");

    assert_eq!(clause.to_string(), "view == 'palette'");
    assert_eq!(clause, when("view == 'palette'"));
}

#[test]
fn a_value_with_a_single_quote_keeps_double_quotes() {
    let clause = when("view == \"it's\"");

    assert_eq!(clause.to_string(), "view == \"it's\"");
    assert_eq!(when(&clause.to_string()), clause);
}

#[test]
fn the_literals_print_as_themselves() {
    assert_eq!(when("true").to_string(), "true");
    assert_eq!(when("false").to_string(), "false");
}

#[test]
fn identifiers_may_hold_dots_digits_and_underscores() {
    let clause = when("config.tab_2.enabled");

    assert_eq!(clause.to_string(), "config.tab_2.enabled");
}

#[test]
fn identifiers_may_start_with_an_underscore() {
    let clause = when("_private");

    assert_eq!(clause.to_string(), "_private");
}

#[test]
fn a_literal_name_compared_to_a_value_is_a_key() {
    let context = WhenContext::new().with_value("true", "yes");

    assert!(holds("true == 'yes'", &context));
    assert!(holds("false != 'yes'", &context));
}

// ---- errors ------------------------------------------------------------------------------------

#[test]
fn an_empty_clause_ends_too_early() {
    assert_eq!(error("  "), WhenError::UnexpectedEnd { offset: 2 });
}

#[test]
fn a_dangling_operator_ends_too_early() {
    assert_eq!(error("a &&"), WhenError::UnexpectedEnd { offset: 4 });
}

#[test]
fn a_leading_operator_is_unexpected() {
    assert_eq!(error("&& a"), WhenError::UnexpectedToken { offset: 0 });
}

#[test]
fn two_identifiers_in_a_row_are_unexpected() {
    assert_eq!(error("a b"), WhenError::UnexpectedToken { offset: 2 });
}

#[test]
fn a_lone_ampersand_is_an_unexpected_character() {
    assert_eq!(
        error("a & b"),
        WhenError::UnexpectedCharacter {
            offset: 2,
            found: '&'
        }
    );
}

#[test]
fn a_lone_bar_is_an_unexpected_character() {
    assert_eq!(
        error("a | b"),
        WhenError::UnexpectedCharacter {
            offset: 2,
            found: '|'
        }
    );
}

#[test]
fn a_lone_equals_sign_is_an_unexpected_character() {
    assert_eq!(
        error("a = 'b'"),
        WhenError::UnexpectedCharacter {
            offset: 2,
            found: '='
        }
    );
}

#[test]
fn an_unknown_character_is_reported_where_it_stands() {
    assert_eq!(
        error("a && #b"),
        WhenError::UnexpectedCharacter {
            offset: 5,
            found: '#'
        }
    );
}

#[test]
fn an_offset_counts_bytes_not_characters() {
    assert_eq!(
        error("view == 'ж' && €"),
        WhenError::UnexpectedCharacter {
            offset: 16,
            found: '€'
        }
    );
}

#[test]
fn an_unclosed_string_is_reported_at_its_quote() {
    assert_eq!(
        error("view == 'palette"),
        WhenError::UnterminatedString { offset: 8 }
    );
}

#[test]
fn a_string_closes_only_with_its_own_quote() {
    assert_eq!(
        error("view == 'palette\""),
        WhenError::UnterminatedString { offset: 8 }
    );
}

#[test]
fn an_unquoted_value_is_unexpected() {
    assert_eq!(
        error("view == palette"),
        WhenError::UnexpectedToken { offset: 8 }
    );
}

#[test]
fn a_missing_value_ends_too_early() {
    assert_eq!(error("view !="), WhenError::UnexpectedEnd { offset: 7 });
}

#[test]
fn a_string_alone_is_unexpected() {
    assert_eq!(error("'palette'"), WhenError::UnexpectedToken { offset: 0 });
}

#[test]
fn an_unclosed_parenthesis_ends_too_early() {
    assert_eq!(error("(a && b"), WhenError::UnexpectedEnd { offset: 7 });
}

#[test]
fn a_parenthesis_closed_by_something_else_is_unexpected() {
    assert_eq!(error("(a b)"), WhenError::UnexpectedToken { offset: 3 });
}

#[test]
fn a_stray_closing_parenthesis_is_unexpected() {
    assert_eq!(error("a)"), WhenError::UnexpectedToken { offset: 1 });
}

#[test]
fn parentheses_may_nest_to_the_limit() {
    let text = format!("{}a{}", "(".repeat(MAX_DEPTH), ")".repeat(MAX_DEPTH));

    assert_eq!(when(&text).to_string(), "a");
}

#[test]
fn parentheses_beyond_the_limit_are_too_deep() {
    let text = format!(
        "{}a{}",
        "(".repeat(MAX_DEPTH + 1),
        ")".repeat(MAX_DEPTH + 1)
    );

    assert_eq!(error(&text), WhenError::TooDeep { offset: MAX_DEPTH });
}

#[test]
fn sibling_groups_do_not_add_up_to_depth() {
    let text = vec!["(a)"; MAX_DEPTH + 1].join(" && ");

    assert!(holds(&text, &flags(&["a"])));
}

#[test]
fn a_long_run_of_nots_parses() {
    let text = format!("{}a", "!".repeat(1000));

    assert!(holds(&text, &flags(&["a"])));
}

#[test]
fn every_error_reports_its_offset() {
    let errors = [
        WhenError::UnexpectedCharacter {
            offset: 1,
            found: '#',
        },
        WhenError::UnterminatedString { offset: 2 },
        WhenError::UnexpectedToken { offset: 3 },
        WhenError::UnexpectedEnd { offset: 4 },
        WhenError::TooDeep { offset: 5 },
    ];

    let offsets: Vec<usize> = errors.iter().map(WhenError::offset).collect();

    assert_eq!(offsets, [1, 2, 3, 4, 5]);
}

#[test]
fn an_error_message_names_the_offset() {
    let message = WhenError::TooDeep { offset: 40 }.to_string();

    assert_eq!(message, "parentheses nest deeper than 32 at offset 40");
}
