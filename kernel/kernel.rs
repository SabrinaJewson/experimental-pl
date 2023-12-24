pub(crate) struct State {
    defs: Vec<(String, Expr)>,
}

pub(crate) mod builtins {
    pub(crate) const LEVEL: Expr = Expr::FVar(0);
    pub(crate) const LEVEL_Z: Expr = Expr::FVar(1);
    pub(crate) const LEVEL_S: Expr = Expr::FVar(2);
    pub(crate) const LEVEL_MAX: Expr = Expr::FVar(3);
    pub(crate) const LEVEL_IMAX: Expr = Expr::FVar(4);
    pub(crate) const SORT: Expr = Expr::FVar(5);
    pub(crate) const BUILTINS: usize = 6;
    use super::*;
}
use builtins::*;

impl State {
    pub fn new(builtin_names: [&str; BUILTINS]) -> Self {
        let builtin_types: [Expr; BUILTINS] = [
            SORT.app([LEVEL_S.app([LEVEL_Z])]),
            LEVEL,
            LEVEL.pi(LEVEL),
            LEVEL.pi(LEVEL).pi(LEVEL),
            LEVEL.pi(LEVEL).pi(LEVEL),
            SORT.app([LEVEL_S.app([Expr::BVar(0)])]).pi(LEVEL),
        ];
        let builtin_names = builtin_names.into_iter().map(str::to_owned);
        let defs = builtin_names.zip(builtin_types).collect();
        State { defs }
    }
    pub fn add(&mut self, name: &str, r#type: Expr) -> u32 {
        self.defs.push((name.to_owned(), r#type));
        (self.defs.len() - 1).try_into().unwrap()
    }
    pub fn truncate(&mut self, len: u32) -> Result<(), String> {
        if len <= 5 {
            return Err("cannot truncate past constants".to_owned());
        }
        self.defs.truncate(len as usize);
        Ok(())
    }
    pub fn type_of(&mut self, value: &Expr) -> Result<Expr, String> {
        let st = self;
        let mut bvars = Vec::new();
        let bvars = Stack::new(&mut bvars);
        let depth = &mut 0;
        type_of(&mut Context { st, bvars, depth }, value)
    }
    pub fn name_of(&self, fvar: u32) -> &str {
        &self.defs[fvar as usize].0
    }
}

struct Context<'a> {
    st: &'a mut State,
    bvars: Stack<'a, &'a Expr>,
    depth: &'a mut u32,
}

fn type_of(cx: &mut Context<'_>, expr: &Expr) -> Result<Expr, String> {
    log::trace!("{:4} type_of({})", cx.display(expr), cx.depth);
    *cx.depth += 1;
    let res = match expr {
        &Expr::FVar(fvar) => cx.st.defs[fvar as usize].1.clone(),
        &Expr::BVar(n) => {
            let mut ty = cx.bvars[cx.bvars.len() - 1 - usize::from(n)].clone();
            (ty.raise(0, n + 1), ty).1
        }
        Expr::Sortω(l) => Expr::Sortω(l.checked_add(1).ok_or("levelω overflow")?),
        Expr::Lam(l, r) => bind(cx, l, |cx, _| Ok(type_of(cx, r)?.pi(l.clone())))?,
        Expr::Pi(l, r) => bind(cx, l, |cx, l_univ| {
            Ok(match (l_univ, type_of(cx, r)?.expect_univ(cx)?) {
                (Univ::Sort(l), Univ::Sort(mut r)) => match r.lower(0, 1) {
                    Ok(()) => SORT.app([LEVEL_IMAX.app([l, r])]),
                    Err(()) => Expr::Sortω(0),
                },
                (Univ::Sortω(a), Univ::Sort(_)) => Expr::Sortω(a),
                (Univ::Sort(_), Univ::Sortω(a)) => Expr::Sortω(a),
                (Univ::Sortω(a), Univ::Sortω(b)) => Expr::Sortω(Ord::max(a, b)),
            })
        })?,
        Expr::App(l, r) => match whnf(type_of(cx, l)?) {
            Expr::Pi(mut f_in, mut f_out) => {
                let mut r_type = type_of(cx, r)?;
                ensure_def_eq(cx, &mut f_in, &mut r_type)?;
                (f_out.subst(r), *f_out).1
            }
            t => {
                let (l, t) = (cx.display(l), cx.display(&t));
                return Err(format!("application LHS `{l} : {t}` not Π type"));
            }
        },
        Expr::Ind(i) => (ind_check(cx, i)?, i.arity.clone()).1,
        &Expr::IndConstr(n, ref i) => {
            ind_check(cx, i)?;
            let c = i.constrs.get(usize::from(n)).cloned();
            let mut c = c.ok_or_else(|| format!("only {} constructors", i.constrs.len()))?;
            (c.subst_with(|e| *e = Expr::Ind(i.clone())), c).1
        }
        Expr::IndElim(i) => {
            ind_check(cx, i)?;
            let univ_params = if i.sm { 0 } else { 1 };
            let constrs = i.constrs.len() as u16;
            let mut t = i.arity.clone();
            t.raise(0, univ_params + 1 + constrs);
            telescope_map(&mut t, 0, |e, d| {
                let mut i = Expr::Ind(i.clone());
                i.raise(0, univ_params + 1 + constrs + d);
                let major_premise = i.app((0..d).rev().map(Expr::BVar));
                let out = Expr::BVar(1 + d + constrs).app((0..=d).rev().map(Expr::BVar));
                *e = out.pi(major_premise);
            });
            for (k, c) in i.constrs.iter().enumerate().rev() {
                let mut minor_premise = c.clone();
                minor_premise.subst_with(|e| *e = Expr::Ind(i.clone()));
                minor_premise.raise(0, univ_params + 1 + k as u16);
                telescope_map(&mut minor_premise, 0, |recs, max_d| {
                    let constr = Expr::IndConstr(k as u16, i.clone());
                    *recs = minor_premise_recs(c, constr, [univ_params, k as u16, max_d, 0, 0])
                });
                t = t.pi(minor_premise);
            }
            let mut motive_type = i.arity.clone();
            motive_type.raise(0, univ_params);
            telescope_map(&mut motive_type, 0, |e, d| {
                let mut ind = Expr::Ind(i.clone());
                ind.raise(0, univ_params + d);
                let v = ind.app((0..d).rev().map(Expr::BVar));
                let rhs = if i.sm { LEVEL_Z } else { Expr::BVar(1 + d) };
                *e = SORT.app([rhs]).pi(v);
            });
            t = t.pi(motive_type);
            t = if i.sm { t } else { t.pi(LEVEL) };
            t
        }
    };
    *cx.depth -= 1;
    log::trace!("{:4} type_of result: {}", cx.depth, cx.display(&res));
    Ok(res)
}

fn ind_check(cx: &mut Context<'_>, ind: &Ind) -> Result<(), String> {
    let mut base_level = arity(cx, &ind.arity, 0)?;
    let level_kind = level_kind(&mut base_level);
    if ind.sm && level_kind != LevelKind::AlwaysZero {
        return Err("small elimination allowed for inductive propositions only".to_owned());
    }

    let mut base_univ = SORT.app([base_level]);
    base_univ.raise(0, 1);
    u16::try_from(ind.constrs.len()).map_err(|_| "too many constructors")?;
    bind(cx, &ind.arity, |cx, _| {
        for c in &ind.constrs {
            let mut univ = type_of(cx, c)?;
            ensure_def_eq(cx, &mut base_univ, &mut univ)?;

            let (resultant_type, max_d) = constr(cx, c, 0)?;
            match level_kind {
                LevelKind::AlwaysZero if ind.sm => {}
                LevelKind::AlwaysNonzero => {}
                _ if 1 < ind.constrs.len() => return Err(">1 constructor".to_owned()),
                _ => {
                    let mut level = univ.into_level().unwrap();
                    singleton(cx, resultant_type, c, 0, max_d, &mut level);
                    let base_level = base_univ.as_level_mut().unwrap();
                    ensure_def_eq(cx, base_level, &mut level)?;
                }
            }
        }
        Ok(())
    })?;

    Ok(())
}

fn minor_premise_recs(c: &Expr, mut constr: Expr, [u, i, max_d, d, rec]: [u16; 5]) -> Expr {
    match c {
        Expr::BVar(_) | Expr::App(_, _) => {
            let mut c = c.clone();
            c.raise(d + 1, u);
            c.raise(d, i);
            c.raise(0, rec);
            constr.raise(0, u + 1 + i + d + rec);
            c.app([constr.app((rec..rec + d).rev().map(Expr::BVar))])
        }
        Expr::Pi(l, c) if l.has_bvar(d) => {
            let mut l = l.clone();
            l.raise(d + 1, u);
            l.raise(d, i);
            l.raise(0, rec + max_d - d);
            telescope_map(&mut l, 0, |e, args| {
                let a = Expr::BVar(args + rec + max_d - 1 - d);
                let a = a.app((0..args).rev().map(Expr::BVar));
                *e = take(e).app([a]);
            });
            minor_premise_recs(c, constr, [u, i, max_d, d + 1, rec + 1]).pi(l)
        }
        Expr::Pi(_, c) => minor_premise_recs(c, constr, [u, i, max_d, d + 1, rec]),
        _ => unreachable!("not a constructor type: {c:?}"),
    }
}

fn minor_premise_rec_args(base: &Expr, c: &Expr, max_d: u16, d: u16, rec: u16, res: &mut Expr) {
    match c {
        Expr::BVar(_) | Expr::App(_, _) => {}
        Expr::Pi(l, c) if l.has_bvar(d) => {
            let mut l = l.clone();
            let mut acc = &mut *res;
            let mut relevant_arg = None;
            for i in 0..rec + max_d {
                let it;
                (acc, it) = acc.unwrap_app();
                match (rec + max_d - 1 - d).cmp(&i) {
                    cmp::Ordering::Less => {
                        l.subst_with(|e| {
                            e.clone_from(it);
                            e.raise(0, rec + max_d - 1 - i);
                        });
                    }
                    cmp::Ordering::Equal => relevant_arg = Some(it),
                    cmp::Ordering::Greater => {}
                }
            }
            l.subst(base);
            telescope_map(&mut l, 0, |e, args| {
                let mut arg = relevant_arg.unwrap().clone();
                arg.raise(0, args);
                let arg = arg.app((0..args).rev().map(Expr::BVar));
                *e = take(e).app([arg]);
            });
            *res = take(res).app([l]);
            minor_premise_rec_args(base, c, max_d, d + 1, rec + 1, res);
        }
        Expr::Pi(_, c) => minor_premise_rec_args(base, c, max_d, d + 1, rec, res),
        _ => unreachable!(),
    }
}

fn arity(cx: &mut Context<'_>, a: &Expr, d: u16) -> Result<Expr, String> {
    if let Some(mut e) = a.as_level().cloned() {
        let msg = "universe level cannot depend on indices";
        (e.lower(0, d).map_err(|()| msg)?, Ok(e)).1
    } else if let Expr::Pi(l, r) = a {
        bind(cx, l, |cx, _| arity(cx, r, d + 1))
    } else {
        Err(format!("{} not a valid arity", cx.display(a)))
    }
}

fn bind<R, F>(cx: &mut Context<'_>, expr: &Expr, f: F) -> Result<R, String>
where
    F: FnOnce(&mut Context<'_>, Univ) -> Result<R, String>,
{
    let univ = type_of(cx, expr)?.expect_univ(cx)?;
    let (st, depth) = (&mut *cx.st, &mut *cx.depth);
    cx.bvars.reborrow().with(expr, move |bvars| {
        let mut cx = Context { st, bvars, depth };
        f(&mut cx, univ)
    })
}

fn ensure_def_eq(cx: &mut Context<'_>, lhs: &mut Expr, rhs: &mut Expr) -> Result<(), String> {
    if !def_eq(cx, lhs, rhs) {
        let (l, r) = (cx.display(lhs), cx.display(rhs));
        return Err(format!("type mismatch:\nexpected {l}\n   found {r}"));
    }
    Ok(())
}

fn def_eq(cx: &mut Context<'_>, lhs: &mut Expr, rhs: &mut Expr) -> bool {
    let (l, r) = (cx.display(lhs), cx.display(rhs));
    log::trace!("{:4} def_eq({l}, {r})", cx.depth);
    *cx.depth += 1;

    make_whnf(lhs);
    make_whnf(rhs);

    let r = (match (&mut *lhs, &mut *rhs) {
        (Expr::FVar(a), Expr::FVar(b)) => a == b,
        (Expr::BVar(n), Expr::BVar(m)) => n == m,
        (Expr::Sortω(n), Expr::Sortω(m)) => n == m,
        (Expr::Pi(a, b), Expr::Pi(c, d)) => {
            def_eq(cx, a, c) && bind(cx, a, |cx, _| Ok(def_eq(cx, b, d))).unwrap()
        }
        (Expr::Lam(a, b), Expr::Lam(c, d)) => {
            def_eq(cx, a, c) && bind(cx, a, |cx, _| Ok(def_eq(cx, b, d))).unwrap()
        }
        (Expr::App(a, b), Expr::App(c, d)) => def_eq(cx, a, c) && def_eq(cx, b, d),
        (Expr::Ind(a), Expr::Ind(b)) => ind_def_eq(cx, a, b),
        (Expr::IndConstr(n, a), Expr::IndConstr(m, b)) => n == m && ind_def_eq(cx, a, b),
        (Expr::IndElim(a), Expr::IndElim(b)) => ind_def_eq(cx, a, b),
        _ => false,
    }) || level::def_eq(cx, lhs, rhs).unwrap_or(false)
        || uip(cx, lhs, rhs);

    *cx.depth -= 1;
    log::trace!("{:4} def_eq result: {r}", cx.depth);

    r
}

fn ind_def_eq(cx: &mut Context<'_>, lhs: &mut Ind, rhs: &mut Ind) -> bool {
    lhs.constrs.len() == rhs.constrs.len()
        && def_eq(cx, &mut lhs.arity, &mut rhs.arity)
        && bind(cx, &lhs.arity, |cx, _| {
            let mut iter = lhs.constrs.iter_mut().zip(&mut rhs.constrs);
            Ok(iter.all(|(l, r)| def_eq(cx, l, r)))
        })
        .unwrap()
}

// TODO: This is inefficient…
fn uip(cx: &mut Context<'_>, lhs: &mut Expr, rhs: &mut Expr) -> bool {
    let not_proof = |e: &Expr| matches!(*e, LEVEL_Z) || e.is_app(&LEVEL_S);
    if not_proof(lhs) || not_proof(rhs) {
        return false;
    }
    let mut lhs_sort = type_of(cx, lhs).unwrap();
    if let Univ::Sort(mut level) = type_of(cx, &lhs_sort).unwrap().into_univ().unwrap() {
        if def_eq(cx, &mut level, &mut LEVEL_Z) {
            let mut rhs_sort = type_of(cx, rhs).unwrap();
            let _ = def_eq(cx, &mut lhs_sort, &mut rhs_sort) && return true;
        }
    }
    false
}

mod level {
    pub(super) fn def_eq(cx: &mut Context<'_>, lhs: &mut Expr, rhs: &mut Expr) -> Result<bool, ()> {
        let _ = (!is(lhs) && !is(rhs)) && return Err(());
        let exprs = Vec::new();
        let mut vars = Vars { cx, exprs };
        let lhs_term = term(&mut vars, lhs)?;
        let rhs_term = term(&mut vars, rhs)?;
        let (mut l, mut r) = Default::default();
        max(&mut l, &lhs_term)?;
        max(&mut r, &rhs_term)?;

        let vars = vars.exprs.len() as u8;
        log::trace!("{} → {l:?}", cx.display(lhs));
        log::trace!("{} → {r:?}", cx.display(rhs));
        let eq = (0..(1_u16 << vars))
            .all(|s| apply(&l, vars, s).is_some_and(|l| Some(l) == apply(&r, vars, s)));
        log::trace!("result: {eq}");
        Ok(eq)
    }
    fn is(e: &Expr) -> bool {
        *e == LEVEL_Z
            || e.is_app(&LEVEL_S)
            || matches!(e, Expr::App(e, _) if e.is_app(&LEVEL_MAX) || e.is_app(&LEVEL_IMAX))
    }
    enum Term {
        Var(u8),
        Zero,
        Succ(Box<Term>),
        Max(Box<Term>, Box<Term>),
        IMax(Box<Term>, Box<Term>),
    }
    struct Vars<'a, 'b, 'e> {
        cx: &'a mut Context<'b>,
        exprs: Vec<&'e mut Expr>,
    }
    fn term<'e>(vars: &mut Vars<'_, '_, 'e>, e: &'e mut Expr) -> Result<Box<Term>, ()> {
        make_whnf(e);
        Ok(Box::new(match e {
            &mut LEVEL_Z => Term::Zero,
            _ if e.is_app(&LEVEL_S) => Term::Succ(term(vars, e.unwrap_app().1)?),
            Expr::App(f, _) if f.is_app(&LEVEL_MAX) => {
                let (f, b) = e.unwrap_app();
                Term::Max(term(vars, f.unwrap_app().1)?, term(vars, b)?)
            }
            Expr::App(f, _) if f.is_app(&LEVEL_IMAX) => {
                let (f, b) = e.unwrap_app();
                Term::IMax(term(vars, f.unwrap_app().1)?, term(vars, b)?)
            }
            _ => Term::Var(match vars
                .exprs
                .iter_mut()
                .position(|ex| super::def_eq(vars.cx, ex, e))
            {
                Some(i) => i,
                None if vars.exprs.len() == 16 => return Err(()),
                None => (vars.exprs.push(e), vars.exprs.len() - 1).1,
            } as u8),
        }))
    }
    type Normalized = Vec<(u16, Vec<(u8, u16)>)>;
    fn max(n: &mut Normalized, t: &Term) -> Result<(), ()> {
        match t {
            &Term::Var(v) => n.push((0, vec![(v, 0)])),
            Term::Zero => n.push((0, Vec::new())),
            Term::Succ(t) => {
                let old_len = n.len();
                max(n, t)?;
                for (k, imax_adds) in &mut n[old_len..] {
                    let k = imax_adds.last_mut().map(|(_, k)| k).unwrap_or(k);
                    *k = k.checked_add(1).ok_or(())?;
                }
            }
            Term::Max(a, b) => (max(n, a)?, max(n, b)?).1,
            Term::IMax(a, b) => imax(n, a, b)?,
        }
        Ok(())
    }
    fn imax(n: &mut Normalized, a: &Term, b: &Term) -> Result<(), ()> {
        match b {
            &Term::Var(b) => {
                let old_len = n.len();
                max(n, a)?;
                n[old_len..].iter_mut().for_each(|(_, v)| v.push((b, 0)));
            }
            Term::Zero => n.push((0, Vec::new())),
            Term::Succ(_) => (max(n, a)?, max(n, b)?).1,
            Term::Max(b, c) => (imax(n, a, b)?, imax(n, a, c)?).1,
            Term::IMax(b, c) => (imax(n, a, c)?, imax(n, b, c)?).1,
        }
        Ok(())
    }
    fn apply(n: &Normalized, vars: u8, states: u16) -> Option<(u16, Vec<u16>)> {
        let mut offsets = vec![0; usize::from(vars)];
        let mut iter = n.iter().map(|&(base, ref imax_adds)| {
            let mut total = 1_u16;
            #[allow(clippy::never_loop, unused_must_use)]
            for &(imax_with, add) in imax_adds.iter().rev() {
                total = total.checked_add(add)?;
                states & (1 << imax_with) == 0 && return Some(total - 1);
                offsets[imax_with as usize] = offsets[imax_with as usize].max(total);
            }
            (total - 1).checked_add(base)
        });
        let k = iter.try_fold(0, |acc, i| Some(acc.max(i?)))?;
        let result = (k.max(*offsets.iter().max().unwrap_or(&0)), offsets);
        log::trace!("{states:16b} → {result:?}");
        Some(result)
    }

    use super::*;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum LevelKind {
    AlwaysZero,
    SometimesZero,
    AlwaysNonzero,
}
pub(crate) fn level_kind(e: &mut Expr) -> LevelKind {
    make_whnf(e);
    match e {
        &mut LEVEL_Z => LevelKind::AlwaysZero,
        Expr::App(f, _) if **f == LEVEL_S => LevelKind::AlwaysNonzero,
        Expr::App(f, b) if f.is_app(&LEVEL_MAX) => {
            Ord::max(level_kind(f.unwrap_app().1), level_kind(b))
        }
        Expr::App(f, b) if f.is_app(&LEVEL_IMAX) => level_kind(b),
        _ => LevelKind::SometimesZero,
    }
}

enum WhnfNext<'e> {
    Ind(usize, usize, u16),
    Lam(&'e mut Expr),
}
fn make_whnf(e: &mut Expr) -> Option<WhnfNext<'_>> {
    Some(loop {
        match e {
            Expr::App(f, arg) => match make_whnf(f)? {
                WhnfNext::Lam(body) => (body.subst(arg), *e = take(body)).1,
                WhnfNext::Ind(0, indices, constrs) => {
                    make_whnf(arg);
                    let mut constr = &mut **arg;
                    let mut max_d = 0;
                    let i = loop {
                        match constr {
                            Expr::App(lhs, _) => (max_d, constr) = (max_d + 1, lhs),
                            &mut Expr::IndConstr(i, _) => break i,
                            _ => return None,
                        }
                    };
                    let mut base = &mut **f;
                    for _ in 0..indices {
                        base = base.unwrap_app().0;
                    }
                    let a = (0..constrs - 1 - i).fold(&mut *base, |acc, _| acc.unwrap_app().0);
                    let constr_sort = match replace(constr, a.unwrap_app().1.clone()) {
                        Expr::IndConstr(_, mut ind) => ind.constrs.swap_remove(i as usize),
                        _ => unreachable!(),
                    };
                    minor_premise_rec_args(base, &constr_sort, max_d, 0, 0, arg);
                    *e = take(arg);
                }
                WhnfNext::Ind(d, indices, constrs) => break WhnfNext::Ind(d - 1, indices, constrs),
            },
            Expr::Lam(_, body) => break WhnfNext::Lam(body),
            Expr::IndElim(i) => {
                let mut a = &mut i.arity;
                let mut indices = 0;
                while let Expr::Pi(_, new_a) = a {
                    (indices, a) = (indices + 1, new_a);
                }
                let depth = if i.sm { 1 } else { 2 } + i.constrs.len() + indices;
                break WhnfNext::Ind(depth, indices, i.constrs.len() as u16);
            }
            _ => return None,
        }
    })
}
fn whnf(mut e: Expr) -> Expr {
    make_whnf(&mut e);
    e
}

fn singleton(cx: &mut Context<'_>, res: &Expr, c: &Expr, d: u16, max_d: u16, level: &mut Expr) {
    if let Expr::Pi(l, c) = c {
        bind(cx, l, |cx, l_univ| {
            let mut acc = res;
            let referenced = loop {
                match acc {
                    Expr::App(_, r) if **r == Expr::BVar(max_d - 1 - d) => break true,
                    Expr::App(new_acc, _) => acc = new_acc,
                    _ => break false,
                }
            };
            if !referenced {
                let mut l_level = l_univ.into_level().unwrap();
                l_level.lower(0, d).unwrap();
                *level = LEVEL_MAX.app([l_level, take(level)]);
            }
            singleton(cx, res, c, d + 1, max_d, level);
            Ok(())
        })
        .unwrap()
    }
}

fn constr<'c>(cx: &Context<'_>, c: &'c Expr, d: u16) -> Result<(&'c Expr, u16), String> {
    Ok(match c {
        &Expr::BVar(v) if v == d => (c, d),
        Expr::App(_, r) if r.has_bvar(d) => return Err("invalid constructor".to_owned()),
        Expr::App(l, _) => (constr(cx, l, d)?, (c, d)).1,
        Expr::Pi(l, r) => {
            let msg = "depended-on parameter cannot reference type";
            match l.has_bvar(d) {
                true if r.has_bvar(0) => return Err(msg.to_owned()),
                true => strict_positive(cx, l, d)?,
                false => {}
            }
            constr(cx, r, d + 1)?
        }
        _ => {
            let c = cx.display(c);
            return Err(format!("invalid expression in constructor: `{c}`"));
        }
    })
}

fn strict_positive(cx: &Context<'_>, e: &Expr, depth: u16) -> Result<(), String> {
    match e {
        &Expr::BVar(v) if v == depth => Ok(()),
        Expr::App(_, r) if r.has_bvar(depth) => Err("not strict positive".to_owned()),
        Expr::App(l, _) => strict_positive(cx, l, depth),
        Expr::Pi(l, r) => {
            if l.has_bvar(depth) {
                return Err("not strict positive".to_owned());
            }
            strict_positive(cx, r, depth + 1)
        }
        _ => Err(format!("not strict positive: `{}`", cx.display(e))),
    }
}

fn telescope_map<R>(e: &mut Expr, d: u16, f: impl FnOnce(&mut Expr, u16) -> R) -> R {
    match e {
        Expr::Pi(_, r) => telescope_map(&mut *r, d + 1, f),
        _ => f(e, d),
    }
}

impl Expr {
    fn has_bvar(&self, n: u16) -> bool {
        Result::is_err(&self.try_visit(n, &mut |n, e| match e {
            &Self::BVar(m) if m == n => Err(()),
            _ => Ok(()),
        }))
    }
    fn subst_with<F: FnMut(&mut Expr)>(&mut self, mut subst: F) {
        self.visit_mut(0, |old, e| match e {
            Self::BVar(n) if old == *n => (subst(e), e.raise(0, old)).1,
            Self::BVar(n) if old < *n => *n -= 1,
            _ => {}
        });
    }
    fn subst(&mut self, new: &Expr) {
        self.subst_with(|e| e.clone_from(new));
    }
    fn raise(&mut self, depth: u16, by: u16) {
        self.visit_mut(depth, |depth, e| match e {
            Self::BVar(n) if depth <= *n => *n += by,
            _ => {}
        })
    }
    fn lower(&mut self, depth: u16, by: u16) -> Result<(), ()> {
        self.try_visit_mut(depth, &mut |depth, e| {
            match e {
                &mut Self::BVar(n) if depth <= n && n < depth + by => return Err(()),
                Self::BVar(n) if depth + by <= *n => *n -= by,
                _ => {}
            }
            Ok(())
        })
    }

    fn into_univ(self) -> Result<Univ, Self> {
        Ok(match self {
            Expr::App(l, r) if *l == SORT => Univ::Sort(r),
            Expr::Sortω(n) => Univ::Sortω(n),
            _ => return Err(self),
        })
    }
    fn into_level(self) -> Result<Expr, Self> {
        match self {
            Expr::App(l, r) if *l == SORT => Ok(*r),
            _ => Err(self),
        }
    }
    fn as_level(&self) -> Option<&Expr> {
        Some(match self {
            Expr::App(l, r) if **l == SORT => r,
            _ => return None,
        })
    }
    fn as_level_mut(&mut self) -> Option<&mut Expr> {
        Some(match self {
            Expr::App(l, r) if **l == SORT => r,
            _ => return None,
        })
    }
    fn expect_univ(self, cx: &Context<'_>) -> Result<Univ, String> {
        self.into_univ()
            .map_err(|e| format!("expression `{}` not a sort", cx.display(&e)))
    }
}
#[derive(Debug)]
pub(crate) enum Univ {
    Sort(Box<Expr>),
    Sortω(u16),
}
impl Univ {
    pub fn into_level(self) -> Result<Expr, Self> {
        match self {
            Self::Sort(level) => Ok(*level),
            Univ::Sortω(_) => Err(self),
        }
    }
}

impl<'c> Context<'c> {
    fn display<'a>(&'a self, e: &'a Expr) -> DisplayExpr<'a, 'c> {
        DisplayExpr(self, e)
    }
}

struct DisplayExpr<'a, 'c>(&'a Context<'c>, &'a Expr);
impl Display for DisplayExpr<'_, '_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self.1 {
            &Expr::FVar(n) => f.write_str(self.0.st.name_of(n)),
            Expr::BVar(n) => write!(f, "_{n}"),
            Expr::Sortω(n) => write!(f, "Sortω{}", Sub(*n)),
            Expr::Lam(l, r) | Expr::Pi(l, r) => {
                let (l_, r) = (Self(self.0, l), Self(self.0, r));
                let s = match &self.1 {
                    Expr::Lam(..) => "λ",
                    _ => "∀",
                };
                match &**l {
                    Expr::Pi(..) | Expr::App(..) => write!(f, "{s} _: ({l_}), {r}"),
                    _ => write!(f, "{s} _: {l_}, {r}"),
                }
            }
            Expr::App(l, r) => {
                match &**l {
                    Expr::Lam(..) => write!(f, "({}) ", Self(self.0, l))?,
                    _ => write!(f, "{} ", Self(self.0, l))?,
                }
                match &**r {
                    Expr::App(..) => write!(f, "({})", Self(self.0, r)),
                    _ => write!(f, "{}", Self(self.0, r)),
                }
            }
            Expr::Ind(i) => write!(f, "Ind{}", DisplayInd(self.0, i)),
            Expr::IndConstr(n, i) => write!(f, "Ind:constr{}{}", Sub(*n), DisplayInd(self.0, i)),
            Expr::IndElim(i) => write!(f, "Ind:elim{}", DisplayInd(self.0, i)),
        }
    }
}

struct DisplayInd<'a, 'c>(&'a Context<'c>, &'a Ind);
impl Display for DisplayInd<'_, '_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self.1.sm {
            false => write!(f, "(_: {}", DisplayExpr(self.0, &self.1.arity))?,
            true => write!(f, "(small, _: {}", DisplayExpr(self.0, &self.1.arity))?,
        }
        for c in &self.1.constrs {
            write!(f, ", {}", DisplayExpr(self.0, c))?;
        }
        f.write_str(")")
    }
}

struct Sub(u16);
impl Display for Sub {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        if self.0 != 0 {
            Display::fmt(&Self(self.0 / 10), f)?;
            write!(f, "{}", self.0 % 10)?;
        }
        Ok(())
    }
}

use crate::expr::Expr;
use crate::expr::Ind;
use crate::stack::Stack;
use std::cmp;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::mem::replace;
use std::mem::take;