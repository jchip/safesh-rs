//! The AST walk. One pass, no execution.

use std::collections::{BTreeMap, BTreeSet};
use std::io::BufReader;
use std::path::Path;

use brush_parser::ast::{
    self, AndOr, Command, CommandPrefixOrSuffixItem as Item, CompoundCommand, CompoundList,
    CompoundListItem, IoFileRedirectKind, IoFileRedirectTarget, IoRedirect,
    ProcessSubstitutionKind, SeparatorOperator,
};
use brush_parser::{Parser, ParserOptions};

use crate::builtins::{is_assign_builtin, is_builtin, is_identifier, is_var_writer};
use crate::path::{is_device, normalize};
use crate::scan::{Scan, assignment_name, literal, scan_compound, scan_list};
use crate::words::Mode;
use crate::{Access, Analysis, AnalyzeOptions, Cwd, Exec, Open, OpenKind, Word};

type Vars = BTreeMap<String, Option<String>>;

const MAX_DEPTH: usize = 40;

#[derive(Clone, PartialEq, Eq)]
enum VarState {
    Unset,
    Unknown,
    Val(String),
}

#[derive(Clone, PartialEq, Eq)]
struct Snap {
    cwd: Cwd,
    ocwd: Cwd,
    lost: bool,
}

struct Branching {
    start_vars: Vars,
    start: Snap,
    moved: bool,
    ends: Vec<Vars>,
}

pub(crate) struct Walker<'a> {
    opts: &'a AnalyzeOptions,
    pub(crate) popts: ParserOptions,
    pub(crate) out: Analysis,
    vars: Vars,
    /// Non-optimistic cwd, used for `Exec.cwd` and `Open.cwd`.
    cwd: Cwd,
    /// Optimistic cwd: every `&&` step is assumed to succeed. Feeds `final_cwd`.
    ocwd: Cwd,
    /// A cwd change the walk could not follow.
    lost: bool,
    fn_assigns: BTreeMap<String, BTreeSet<String>>,
    cwd_fns: BTreeSet<String>,
    depth: usize,
    may_change: bool,
}

pub(crate) fn parse_script(
    src: &str,
    popts: &ParserOptions,
) -> Result<ast::Program, brush_parser::ParseError> {
    Parser::new(BufReader::new(src.as_bytes()), popts).parse_program()
}

fn is_fd_target(raw: &str) -> bool {
    let t = raw.strip_suffix('-').unwrap_or(raw);
    raw == "-" || (!t.is_empty() && t.chars().all(|c| c.is_ascii_digit()))
}

impl<'a> Walker<'a> {
    pub(crate) fn new(opts: &'a AnalyzeOptions) -> Self {
        let base = Cwd::Known(normalize(&opts.base_cwd));
        Self {
            opts,
            popts: ParserOptions::default(),
            out: Analysis::default(),
            vars: Vars::new(),
            cwd: base.clone(),
            ocwd: base,
            lost: false,
            fn_assigns: BTreeMap::new(),
            cwd_fns: BTreeSet::new(),
            depth: 0,
            may_change: false,
        }
    }

    pub(crate) fn finish(mut self) -> Analysis {
        self.out.may_change_cwd = self.may_change;
        self.out.final_cwd = match (&self.ocwd, self.lost) {
            (Cwd::Known(p), false) => Some(p.clone()),
            _ => None,
        };
        self.out
    }

    // ---- state helpers ----

    pub(crate) fn var_value(&self, name: &str) -> Option<String> {
        match self.var_state(name) {
            VarState::Val(v) => Some(v),
            _ => None,
        }
    }

    fn var_state(&self, name: &str) -> VarState {
        match self.vars.get(name) {
            Some(Some(v)) => VarState::Val(v.clone()),
            Some(None) => VarState::Unknown,
            None => match self.opts.env.get(name) {
                Some(v) => VarState::Val(v.clone()),
                None => VarState::Unset,
            },
        }
    }

    pub(crate) fn vars_set_unknown(&mut self, name: &str) {
        self.vars.insert(name.to_string(), None);
    }

    fn snap(&self) -> Snap {
        Snap {
            cwd: self.cwd.clone(),
            ocwd: self.ocwd.clone(),
            lost: self.lost,
        }
    }

    fn restore_cwd(&mut self, s: &Snap) {
        self.cwd = s.cwd.clone();
        self.ocwd = s.ocwd.clone();
    }

    fn invalidate_cwd(&mut self) {
        self.cwd = Cwd::Unknown;
        self.ocwd = Cwd::Unknown;
    }

    fn lose_cwd(&mut self) {
        self.invalidate_cwd();
        self.lost = true;
    }

    /// Run `f` on a copy of the variable table and cwd.
    fn isolated(&mut self, f: impl FnOnce(&mut Self)) {
        let vars = self.vars.clone();
        let snap = self.snap();
        f(self);
        self.vars = vars;
        self.restore_cwd(&snap);
        self.lost = snap.lost;
    }

    pub(crate) fn unanalyzable(&mut self, text: &str) {
        self.out.execs.push(Exec {
            argv: vec![Word::Unknown(text.to_string())],
            cwd: self.cwd.clone(),
            builtin: false,
            function: false,
        });
    }

    // ---- branching ----

    fn branch_begin(&self) -> Branching {
        Branching {
            start_vars: self.vars.clone(),
            start: self.snap(),
            moved: false,
            ends: Vec::new(),
        }
    }

    fn branch_next(&mut self, b: &Branching) {
        self.vars = b.start_vars.clone();
        self.restore_cwd(&b.start);
    }

    fn branch_end(&mut self, b: &mut Branching) {
        if self.snap() != b.start {
            b.moved = true;
        }
        b.ends.push(std::mem::take(&mut self.vars));
    }

    fn branch_finish(&mut self, b: Branching) {
        self.vars = b.start_vars.clone();
        for end in b.ends {
            self.merge_vars(&b.start_vars, end);
        }
        self.restore_cwd(&b.start);
        if b.moved {
            self.invalidate_cwd();
        }
    }

    /// Fold a conditionally run branch back in. A name keeps the new value only when it was
    /// unset before or already equal. Otherwise it becomes unknown.
    fn merge_vars(&mut self, start: &Vars, end: Vars) {
        for (k, v) in end {
            if start.get(&k) == Some(&v) {
                continue;
            }
            let merged = match (self.var_state(&k), &v) {
                (VarState::Unset, _) => v,
                (VarState::Val(old), Some(new)) if &old == new => v,
                _ => None,
            };
            self.vars.insert(k, merged);
        }
    }

    // ---- entry ----

    pub(crate) fn program(&mut self, p: &ast::Program) {
        for l in &p.complete_commands {
            self.list(l);
        }
    }

    fn list(&mut self, l: &CompoundList) {
        for CompoundListItem(ao, sep) in &l.0 {
            if matches!(sep, SeparatorOperator::Async) {
                self.isolated(|w| w.and_or(ao));
            } else {
                self.and_or(ao);
            }
        }
    }

    fn and_or(&mut self, ao: &ast::AndOrList) {
        if ao.additional.is_empty() {
            self.pipeline(&ao.first);
            return;
        }
        let pre = self.snap();
        self.pipeline(&ao.first);
        let after_first = self.vars.clone();
        let mut has_or = false;
        for item in &ao.additional {
            let (is_or, p) = match item {
                AndOr::And(p) => (false, p),
                AndOr::Or(p) => (true, p),
            };
            if is_or {
                has_or = true;
                if self.cwd != pre.cwd {
                    self.cwd = Cwd::Unknown;
                }
            }
            self.pipeline(p);
        }
        let end = std::mem::replace(&mut self.vars, after_first.clone());
        self.merge_vars(&after_first, end);
        if self.snap() != pre {
            self.cwd = Cwd::Unknown;
            if has_or {
                self.ocwd = Cwd::Unknown;
            }
        }
    }

    fn pipeline(&mut self, p: &ast::Pipeline) {
        if p.seq.len() == 1 {
            self.command(&p.seq[0]);
        } else {
            let slot = self.out.pipelines.len();
            self.out.pipelines.push(Vec::new());
            let mut stages = Some(Vec::new());
            for c in &p.seq {
                self.isolated(|w| {
                    if let Command::Simple(sc) = c {
                        match (w.simple(sc), stages.as_mut()) {
                            (Some(i), Some(v)) => v.push(i),
                            _ => stages = None,
                        }
                    } else {
                        w.command(c);
                        stages = None;
                    }
                });
            }
            match stages {
                Some(v) => self.out.pipelines[slot] = v,
                None => {
                    self.out.pipelines.remove(slot);
                }
            }
        }
    }

    fn command(&mut self, c: &Command) {
        match c {
            Command::Simple(sc) => {
                self.simple(sc);
            }
            Command::Compound(cc, redirs) => {
                for r in redirs.iter().flat_map(|l| l.0.iter()) {
                    self.redirect(r);
                }
                self.compound(cc);
            }
            Command::Function(f) => self.function(f),
        }
    }

    fn function(&mut self, f: &ast::FunctionDefinition) {
        let name = f.fname.value.clone();
        self.out.functions.insert(name.clone());
        let mut scan = Scan::default();
        scan_compound(&f.body.0, &mut scan);
        let (assigned, moves) = self.summarize(&scan);
        self.fn_assigns.insert(name.clone(), assigned);
        if moves {
            self.cwd_fns.insert(name);
        }
        self.isolated(|w| {
            w.invalidate_cwd();
            for r in f.body.1.iter().flat_map(|l| l.0.iter()) {
                w.redirect(r);
            }
            w.compound(&f.body.0);
        });
    }

    /// Variables a subtree may write and whether it may change the cwd, including through
    /// functions defined so far.
    fn summarize(&self, scan: &Scan) -> (BTreeSet<String>, bool) {
        let mut assigned = scan.assigned.clone();
        let mut moves = scan.dynamic;
        for n in &scan.names {
            if matches!(
                n.as_str(),
                "cd" | "pushd" | "popd" | "source" | "." | "eval" | "command" | "builtin"
            ) || self.cwd_fns.contains(n)
            {
                moves = true;
            }
            if let Some(a) = self.fn_assigns.get(n) {
                assigned.extend(a.iter().cloned());
            }
        }
        (assigned, moves)
    }

    fn compound(&mut self, cc: &CompoundCommand) {
        match cc {
            CompoundCommand::Arithmetic(a) => self.arith_text(&a.expr.value),
            CompoundCommand::ArithmeticForClause(f) => {
                for e in [&f.initializer, &f.condition, &f.updater]
                    .into_iter()
                    .flatten()
                {
                    self.arith_text(&e.value);
                }
                self.looped(&[&f.body.list], |w| w.list(&f.body.list));
            }
            CompoundCommand::BraceGroup(b) => self.list(&b.list),
            CompoundCommand::Subshell(b) => self.isolated(|w| w.list(&b.list)),
            CompoundCommand::ForClause(f) => {
                for v in f.values.iter().flatten() {
                    self.expand(&v.value, Mode::Arg);
                }
                self.vars_set_unknown(&f.variable_name);
                self.looped(&[&f.body.list], |w| w.list(&f.body.list));
            }
            CompoundCommand::CaseClause(c) => self.case(c),
            CompoundCommand::IfClause(i) => self.if_clause(i),
            CompoundCommand::WhileClause(w) | CompoundCommand::UntilClause(w) => {
                self.looped(&[&w.0, &w.1.list], |s| {
                    s.list(&w.0);
                    s.list(&w.1.list);
                });
            }
            CompoundCommand::Coprocess(c) => {
                if let Some(n) = &c.name {
                    self.expand(&n.value, Mode::Arg);
                }
                self.isolated(|w| w.command(&c.body));
            }
            CompoundCommand::ExtendedTest(t) => self.test_expr(&t.expr),
        }
    }

    fn test_expr(&mut self, e: &ast::ExtendedTestExpr) {
        use ast::ExtendedTestExpr as E;
        match e {
            E::And(a, b) | E::Or(a, b) => {
                self.test_expr(a);
                self.test_expr(b);
            }
            E::Not(a) | E::Parenthesized(a) => self.test_expr(a),
            E::UnaryTest(_, w) => {
                self.expand(&w.value, Mode::Arg);
            }
            E::BinaryTest(_, a, b) => {
                self.expand(&a.value, Mode::Arg);
                self.expand(&b.value, Mode::Arg);
            }
        }
    }

    /// Loop body: invalidate what the loop may write and start from an unknown cwd when the
    /// loop may change it, since later iterations see the earlier ones.
    fn looped(&mut self, lists: &[&CompoundList], run: impl FnOnce(&mut Self)) {
        let mut scan = Scan::default();
        for l in lists {
            scan_list(l, &mut scan);
        }
        // A name that is only known at run time is judged by the walk: it resolves, or it
        // loses the cwd by itself.
        let dynamic = std::mem::take(&mut scan.dynamic);
        let (assigned, moves) = self.summarize(&scan);
        for n in &assigned {
            self.vars_set_unknown(n);
        }
        if moves {
            self.invalidate_cwd();
        }
        let before = (self.cwd.clone(), self.ocwd.clone());
        run(self);
        if dynamic && (self.cwd.clone(), self.ocwd.clone()) != before {
            self.invalidate_cwd();
        }
        for n in &assigned {
            self.vars_set_unknown(n);
        }
        if moves {
            self.invalidate_cwd();
        }
    }

    fn if_clause(&mut self, i: &ast::IfClauseCommand) {
        let pre = self.snap();
        self.list(&i.condition);
        let cond_moved = self.snap() != pre;
        let mut b = self.branch_begin();
        b.moved = cond_moved;
        self.branch_next(&b);
        self.list(&i.then);
        self.branch_end(&mut b);
        for e in i.elses.iter().flatten() {
            self.branch_next(&b);
            if cond_moved {
                self.invalidate_cwd();
            }
            if let Some(c) = &e.condition {
                self.list(c);
            }
            self.list(&e.body);
            self.branch_end(&mut b);
        }
        self.branch_finish(b);
    }

    fn case(&mut self, c: &ast::CaseClauseCommand) {
        self.expand(&c.value.value, Mode::Arg);
        for item in &c.cases {
            for p in &item.patterns {
                self.expand(&p.value, Mode::Arg);
            }
        }
        let mut b = self.branch_begin();
        for item in &c.cases {
            self.branch_next(&b);
            if let Some(l) = &item.cmd {
                self.list(l);
            }
            self.branch_end(&mut b);
        }
        self.branch_finish(b);
    }

    // ---- substitutions ----

    pub(crate) fn cmdsub(&mut self, src: &str) {
        if self.depth >= MAX_DEPTH {
            self.unanalyzable(src);
            return;
        }
        match parse_script(src, &self.popts) {
            Ok(p) => {
                self.depth += 1;
                self.isolated(|w| w.program(&p));
                self.depth -= 1;
            }
            Err(_) => self.unanalyzable(src),
        }
    }

    // ---- redirects ----

    fn redirect(&mut self, r: &IoRedirect) {
        match r {
            IoRedirect::File(_, kind, target) => match target {
                IoFileRedirectTarget::Filename(w) => {
                    self.open_redirect(&w.value, redirect_access(kind));
                }
                IoFileRedirectTarget::Fd(_) => {}
                IoFileRedirectTarget::ProcessSubstitution(_, sub) => {
                    self.isolated(|w| w.list(&sub.list));
                }
                IoFileRedirectTarget::Duplicate(w) => {
                    if !is_fd_target(&w.value) {
                        self.open_redirect(&w.value, redirect_access(kind));
                    }
                }
            },
            IoRedirect::HereDocument(_, h) => {
                if h.requires_expansion {
                    self.walk_heredoc(&h.doc.value);
                }
            }
            IoRedirect::HereString(_, w) => {
                self.expand(&w.value, Mode::Assign);
            }
            IoRedirect::OutputAndError(w, _) => self.open_redirect(&w.value, Access::Write),
        }
    }

    fn open_redirect(&mut self, raw: &str, access: Access) {
        let path = self.expand(raw, Mode::Arg);
        if let Word::Known(p) = &path
            && is_device(p)
        {
            return;
        }
        self.out.opens.push(Open {
            path,
            access,
            kind: OpenKind::Redirect,
            cwd: self.cwd.clone(),
        });
    }

    // ---- simple commands ----

    /// Walk a simple command. Returns the index of its `Exec` when it produced one.
    fn simple(&mut self, sc: &ast::SimpleCommand) -> Option<usize> {
        let has_cmd = sc.word_or_name.is_some();
        for it in sc.prefix.iter().flat_map(|p| p.0.iter()) {
            match it {
                Item::AssignmentWord(a, _) => {
                    let v = self.assignment_value(a);
                    if !has_cmd {
                        self.set_var(a, v);
                    }
                }
                Item::IoRedirect(r) => self.redirect(r),
                Item::ProcessSubstitution(_, sub) => self.isolated(|w| w.list(&sub.list)),
                Item::Word(w) => {
                    self.expand(&w.value, Mode::Arg);
                }
            }
        }
        let name = sc.word_or_name.as_ref()?;
        let name_word = self.expand(&name.value, Mode::Arg);
        let at = self.out.execs.len();
        let mut argv = vec![name_word];
        let mut assigns: Vec<(&ast::Assignment, Option<String>)> = Vec::new();
        let mut raw_args: Vec<&str> = Vec::new();
        for it in sc.suffix.iter().flat_map(|p| p.0.iter()) {
            match it {
                Item::Word(w) => {
                    argv.push(self.expand(&w.value, Mode::Arg));
                    raw_args.push(&w.value);
                }
                Item::AssignmentWord(a, w) => {
                    argv.push(self.expand(&w.value, Mode::Assign));
                    let v = self.assignment_value(a);
                    assigns.push((a, v));
                }
                Item::IoRedirect(r) => self.redirect(r),
                Item::ProcessSubstitution(kind, sub) => {
                    self.isolated(|w| w.list(&sub.list));
                    let t = match kind {
                        ProcessSubstitutionKind::Read => "<(...)",
                        ProcessSubstitutionKind::Write => ">(...)",
                    };
                    argv.push(Word::Unknown(t.to_string()));
                }
            }
        }
        let exec = self.dispatch(argv, &assigns, &raw_args);
        let e = exec?;
        let at = at.min(self.out.execs.len());
        self.out.execs.insert(at, e);
        // Sub-commands walked while expanding the arguments moved up by one.
        for i in self.out.pipelines.iter_mut().flatten() {
            if *i >= at {
                *i += 1;
            }
        }
        Some(at)
    }

    /// Value of an assignment as a string, or `None` when unknown. Walks substitutions.
    fn assignment_value(&mut self, a: &ast::Assignment) -> Option<String> {
        match &a.value {
            ast::AssignmentValue::Scalar(w) => {
                let v = self.expand(&w.value, Mode::Assign);
                if a.append || matches!(a.name, ast::AssignmentName::ArrayElementName(..)) {
                    // Appending needs the old value. Keep it simple and only resolve plain sets.
                    let Word::Known(new) = v else { return None };
                    if matches!(a.name, ast::AssignmentName::ArrayElementName(..)) {
                        return None;
                    }
                    return self.var_value(assignment_name(a)).map(|old| old + &new);
                }
                match v {
                    Word::Known(s) => Some(s),
                    Word::Unknown(_) => None,
                }
            }
            ast::AssignmentValue::Array(items) => {
                for (k, v) in items {
                    if let Some(k) = k {
                        self.expand(&k.value, Mode::Assign);
                    }
                    self.expand(&v.value, Mode::Assign);
                }
                None
            }
        }
    }

    fn set_var(&mut self, a: &ast::Assignment, v: Option<String>) {
        self.vars.insert(assignment_name(a).to_string(), v);
    }

    /// Record the exec for a fully expanded command and apply its shell side effects.
    fn dispatch(
        &mut self,
        argv: Vec<Word>,
        assigns: &[(&ast::Assignment, Option<String>)],
        raw_args: &[&str],
    ) -> Option<Exec> {
        let mut argv = argv;
        let mut bypass_fn = false;
        let mut as_exec = false;
        while let Some(name) = known_name(&argv[0]) {
            match name.as_str() {
                "command" => {
                    let (skip, query) = command_options(&argv);
                    if query || skip >= argv.len() {
                        break;
                    }
                    argv.drain(..skip);
                    bypass_fn = true;
                }
                "builtin" if argv.len() > 1 => {
                    argv.remove(0);
                    bypass_fn = true;
                }
                "exec" => {
                    let skip = exec_options(&argv);
                    if skip >= argv.len() {
                        // Redirect-only exec: the redirects were already recorded.
                        return None;
                    }
                    argv.drain(..skip);
                    bypass_fn = true;
                    as_exec = true;
                }
                _ => break,
            }
        }
        let cwd = self.cwd.clone();
        let Some(name) = known_name(&argv[0]) else {
            self.may_change = true;
            self.lose_cwd();
            return Some(Exec {
                argv,
                cwd,
                builtin: false,
                function: false,
            });
        };
        let function = !bypass_fn && self.out.functions.contains(&name);
        let builtin = !function && !as_exec && is_builtin(&name);
        if function {
            if let Some(a) = self.fn_assigns.get(&name).cloned() {
                for n in a {
                    self.vars_set_unknown(&n);
                }
            }
            if self.cwd_fns.contains(&name) {
                self.may_change = true;
                self.lose_cwd();
            }
        }
        if builtin {
            self.builtin_effects(&name, &argv, assigns, raw_args);
        }
        Some(Exec {
            argv,
            cwd,
            builtin,
            function,
        })
    }

    fn builtin_effects(
        &mut self,
        name: &str,
        argv: &[Word],
        assigns: &[(&ast::Assignment, Option<String>)],
        raw_args: &[&str],
    ) {
        match name {
            "cd" => self.change_dir(&argv[1..]),
            "pushd" | "popd" | "eval" => {
                self.may_change = true;
                self.lose_cwd();
            }
            "source" | "." => {
                self.may_change = true;
                if let Some(p) = argv.get(1) {
                    self.out.opens.push(Open {
                        path: p.clone(),
                        access: Access::Read,
                        kind: OpenKind::Source,
                        cwd: self.cwd.clone(),
                    });
                }
                self.lose_cwd();
            }
            n if is_assign_builtin(n) => {
                for (a, v) in assigns {
                    self.set_var(a, v.clone());
                }
            }
            n if is_var_writer(n) => {
                for r in raw_args {
                    if let Some(l) = literal(r) {
                        if is_identifier(&l) {
                            self.vars_set_unknown(&l);
                        }
                        for t in crate::scan::arith_targets(&l) {
                            self.vars_set_unknown(&t);
                        }
                    }
                }
                for (a, _) in assigns {
                    self.vars_set_unknown(assignment_name(a));
                }
            }
            _ => {}
        }
    }

    fn change_dir(&mut self, args: &[Word]) {
        self.may_change = true;
        let mut i = 0;
        while let Some(Word::Known(s)) = args.get(i) {
            if s == "--" {
                i += 1;
                break;
            }
            if s.len() > 1 && s.starts_with('-') {
                i += 1;
            } else {
                break;
            }
        }
        match args.get(i) {
            None => match self.var_value("HOME") {
                Some(h) if h.starts_with('/') => self.set_cwd(&h),
                _ => self.lose_cwd(),
            },
            Some(Word::Known(p)) if p != "-" => {
                let p = p.clone();
                self.set_cwd(&p);
            }
            Some(_) => self.lose_cwd(),
        }
    }

    fn set_cwd(&mut self, target: &str) {
        if target.is_empty() {
            return;
        }
        let apply = |cur: &Cwd| match cur {
            _ if Path::new(target).is_absolute() => Cwd::Known(normalize(Path::new(target))),
            Cwd::Known(c) => Cwd::Known(normalize(&c.join(target))),
            Cwd::Unknown => Cwd::Unknown,
        };
        self.cwd = apply(&self.cwd);
        self.ocwd = apply(&self.ocwd);
    }
}

fn redirect_access(kind: &IoFileRedirectKind) -> Access {
    match kind {
        IoFileRedirectKind::Read | IoFileRedirectKind::DuplicateInput => Access::Read,
        IoFileRedirectKind::Write
        | IoFileRedirectKind::Append
        | IoFileRedirectKind::ReadAndWrite
        | IoFileRedirectKind::Clobber
        | IoFileRedirectKind::DuplicateOutput => Access::Write,
    }
}

/// Command name when known, non-empty and a single word.
fn known_name(w: &Word) -> Option<String> {
    match w {
        Word::Known(s) if !s.is_empty() && !s.contains(char::is_whitespace) => Some(s.clone()),
        _ => None,
    }
}

/// Number of leading words of `command ...` to drop, and whether it is a `-v`/`-V` query.
fn command_options(argv: &[Word]) -> (usize, bool) {
    let mut i = 1;
    let mut query = false;
    while let Some(Word::Known(s)) = argv.get(i) {
        if s == "--" {
            i += 1;
            break;
        }
        if s.len() > 1 && s.starts_with('-') {
            query |= s.contains(['v', 'V']);
            i += 1;
        } else {
            break;
        }
    }
    (i, query)
}

/// Number of leading words of `exec ...` that are options.
fn exec_options(argv: &[Word]) -> usize {
    let mut i = 1;
    while let Some(Word::Known(s)) = argv.get(i) {
        if s == "--" {
            return i + 1;
        }
        if s == "-a" {
            i += 2;
        } else if s.len() > 1 && s.starts_with('-') {
            i += 1;
        } else {
            break;
        }
    }
    i
}
