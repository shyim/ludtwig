use std::borrow::Borrow;

use codespan_reporting::diagnostic::{Diagnostic, Label};
use codespan_reporting::files::SimpleFiles;
use codespan_reporting::term;
use codespan_reporting::term::termcolor::Buffer;

use ludtwig_parser::syntax::typed;
use ludtwig_parser::syntax::typed::{AstNode, HtmlTag, LudtwigDirectiveIgnore};
use ludtwig_parser::syntax::untyped::{debug_tree, SyntaxElement, SyntaxToken, WalkEvent};

use crate::check::rule::{CheckSuggestion, RuleContext, Severity, TreeTraversalContext};
use crate::process::FileContext;
use crate::ProcessingEvent;

pub mod rule;
pub mod rules;

pub fn run_rules(file_context: &FileContext) -> RuleContext {
    let mut ctx = RuleContext {
        check_results: vec![],
        cli_context: file_context.cli_context.clone(),
        traversal_ctx: TreeTraversalContext {
            inside_trivia_sensitive_node: false,
        },
    };

    if file_context.file_rule_definitions.is_empty() {
        // no rules to run for this file
        return ctx;
    }

    // run root node checks once for each rule
    for rule in &file_context.file_rule_definitions {
        rule.check_root(file_context.tree_root.clone(), &mut ctx);
    }

    // iterate through syntax tree
    let mut ignored_rules: Vec<String> = vec![];
    let mut preorder = file_context.tree_root.preorder_with_tokens();
    while let Some(walk_event) = preorder.next() {
        match walk_event {
            WalkEvent::Enter(element) => {
                // add ignored rules when entering the sibling after the ignore directive / comment
                // also skip the whole subtree if there is a ignore directive without specific rules
                let mut found_ignored_rules: Vec<String> = match element.prev_sibling_or_token() {
                    Some(SyntaxElement::Node(node)) => match LudtwigDirectiveIgnore::cast(node) {
                        Some(directive) => {
                            let ignored_rules = directive.get_rules();
                            if ignored_rules.is_empty() {
                                // all rules are disabled
                                preorder.skip_subtree();
                                continue;
                            }

                            ignored_rules
                        }
                        None => vec![],
                    },
                    _ => vec![],
                };
                ignored_rules.append(&mut found_ignored_rules);

                // actually run the rules
                match element {
                    SyntaxElement::Node(n) => {
                        if typed::Error::can_cast(n.kind()) {
                            preorder.skip_subtree();
                            continue; // Skip error nodes in rules because they should have
                                      // corresponding parser error messages in most cases and can contain
                                      // malformed code
                        }

                        // adjust traversal context when entering special nodes
                        if let Some(t) = HtmlTag::cast(n.clone()) {
                            if let Some("pre" | "textarea") =
                                t.name().as_ref().map(SyntaxToken::text)
                            {
                                ctx.traversal_ctx.inside_trivia_sensitive_node = true;
                            }
                        }

                        // run node checks for every rule
                        for rule in &file_context.file_rule_definitions {
                            if !ignored_rules.iter().any(|ignored| ignored == rule.name()) {
                                rule.check_node(n.clone(), &mut ctx);
                            }
                        }
                    }
                    SyntaxElement::Token(t) => {
                        // run token checks for every rule
                        for rule in &file_context.file_rule_definitions {
                            if !ignored_rules.iter().any(|ignored| ignored == rule.name()) {
                                rule.check_token(t.clone(), &mut ctx);
                            }
                        }
                    }
                }
            }
            WalkEvent::Leave(element) => {
                // remove ignored rules when leaving the sibling after the ignore directive / comment
                let found_ignored_rules: Vec<String> = match element.prev_sibling_or_token() {
                    Some(SyntaxElement::Node(node)) => match LudtwigDirectiveIgnore::cast(node) {
                        Some(directive) => directive.get_rules(),
                        None => vec![],
                    },
                    _ => vec![],
                };

                for rule in found_ignored_rules {
                    ignored_rules.remove(ignored_rules.iter().position(|r| r == &rule).unwrap());
                }

                // adjust traversal context when exiting special nodes
                if let SyntaxElement::Node(n) = element {
                    if let Some(t) = HtmlTag::cast(n) {
                        if let Some("pre" | "textarea") = t.name().as_ref().map(SyntaxToken::text) {
                            ctx.traversal_ctx.inside_trivia_sensitive_node = false;
                        }
                    }
                }
            }
        }
    }

    ctx
}

pub fn get_rule_context_suggestions(rule_ctx: &RuleContext) -> Vec<(&str, &CheckSuggestion)> {
    rule_ctx
        .check_results
        .iter()
        .flat_map(|res| {
            let rule_name = res.rule_name.borrow();
            res.suggestions.iter().map(move |sug| (rule_name, sug))
        })
        .collect()
}

pub fn produce_diagnostics(
    file_context: &FileContext,
    result_rule_ctx: RuleContext,
    buffer: &mut Buffer,
) {
    // diagnostic output setup
    let mut files = SimpleFiles::new();
    let file_id = files.add(
        file_context.file_path.to_string_lossy(),
        &file_context.source_code,
    );
    let config = term::Config {
        // styles: Styles::with_blue(term::termcolor::Color::Cyan),
        ..Default::default()
    };

    if file_context.cli_context.data.inspect {
        // notify output about this
        file_context.send_processing_output(ProcessingEvent::Report(Severity::Info));

        let diagnostic = Diagnostic::note()
            .with_code("SyntaxTree")
            .with_message("visualization of the syntax tree (inspect cli option is active)")
            .with_notes(vec![debug_tree(&file_context.tree_root)]);

        term::emit(buffer, &config, &files, &diagnostic).unwrap();
    }

    // run through the parser errors
    for result in &file_context.parse_errors {
        // notify output about this
        file_context.send_processing_output(ProcessingEvent::Report(Severity::Error));
        let label = Label::primary(file_id, result.range).with_message(result.expected_message());
        let diagnostic = Diagnostic::error()
            .with_code("SyntaxError")
            .with_message("The parser encountered a syntax error")
            .with_labels(vec![label]);

        term::emit(buffer, &config, &files, &diagnostic).unwrap();
    }

    // run through the rule check results
    for result in result_rule_ctx.check_results {
        let diagnostic = match result.severity {
            Severity::Error => Diagnostic::error(),
            Severity::Warning => Diagnostic::warning(),
            Severity::Help => Diagnostic::help(),
            Severity::Info => Diagnostic::note(),
        };

        // notify output about this
        file_context.send_processing_output(ProcessingEvent::Report(result.severity));

        let mut labels = vec![];
        if let Some(primary) = result.primary {
            labels
                .push(Label::primary(file_id, primary.syntax_range).with_message(primary.message));
        }

        for suggestion in result.suggestions {
            labels.push(
                Label::secondary(file_id, suggestion.syntax_range).with_message(format!(
                    "{}: {}",
                    suggestion.message, suggestion.replace_with
                )),
            );
        }

        let diagnostic = diagnostic
            .with_code(result.rule_name)
            .with_message(result.message)
            .with_labels(labels);

        term::emit(buffer, &config, &files, &diagnostic).unwrap();
    }
}
