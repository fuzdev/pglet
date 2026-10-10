-- A `::regclass` value prints the relation's name **rendered for an identifier
-- position**, not its stored bytes. 18.6's `regclassout` ends in
-- `quote_qualified_identifier`, so a relation created as `"Quo Odd"` reads back with its
-- quotes; pglet answered the bare `Quo Odd`, which is not a spelling that names this
-- relation anywhere — `SELECT 'Quo Odd'::regclass` is `42602 invalid name syntax` on
-- 18.6, its name-literal grammar refusing an unquoted word pair. It renders through
-- `lexer::quote_ident` now, the one `fmtId`-style predicate this engine has. The **round
-- trip** the rule buys — the answer fed back to `'…'::regclass` naming the same
-- relation, where the stored-bytes answer is
-- `42P01` on both engines — is pinned over the `"Odd"` spelling in
-- `crates/pglet/tests/regclass_cast_target.rs`, a second statement this fixture's single
-- `@test` has no room for.
--
-- Both argument arms are projected, because they are two `match` arms and only the pair
-- sees a fix applied to one: the string-literal arm (columns 1-3) and the OID arm
-- (column 5, reached through `pg_class`; column 4 names which relation the row is for).
-- The **bare-named** relation beside each is the over-correction control — a renderer
-- that quoted unconditionally reddens on `quo_plain` in both arms while still passing
-- `"Quo Odd"`.
--
-- `{quo_plain}` is a third spelling and not a fourth relation-name shape: both engines
-- read a `::regclass` literal through their name-literal grammar
-- (`stringToQualifiedNameList` there, `parser::reduce_name_literal` here), which takes
-- `{quo_plain}` as one identifier rather than as an array literal — so the *cast target*
-- decides whether the braces are data or a name, which is
-- `regclass_array_target_is_loud.sql`'s subject and this column's control for it.
--
-- The `::text` casts are the corpus idiom for `::regclass` (see
-- `regclass_resolves_a_relation_name.sql`): a bare `<expr>::regclass` is declared `text`
-- here and `regclass` (2205) there, a divergence outside this fixture's subject.
-- @setup
CREATE TABLE quo_plain (a INT);
CREATE TABLE "Quo Odd" (a INT);
CREATE TABLE "{quo_plain}" (a INT);

-- @test
SELECT '"Quo Odd"'::regclass::text AS quoted_literal,
       'quo_plain'::regclass::text AS plain_literal,
       '{quo_plain}'::regclass::text AS brace_literal,
       c.relname::text AS relname,
       c.oid::regclass::text AS from_oid
FROM pg_class c
WHERE c.relname IN ('Quo Odd', 'quo_plain')
ORDER BY 4
