-- An **array** cast target is not the `regclass` model. `parser::expr`'s
-- `parse_cast_target` decides on the canonical type name now, where it used to peek the
-- identifier `regclass` and discard what `parse_type_name` returned — a reader that also
-- consumes a trailing `[]` and a `(…)` modifier, so `'x'::regclass[]`, `'x'::regclass(3)`
-- and `CAST('x' AS regclass[])` all built the node `'x'::regclass` builds. That answered
-- a relation name where 18.6 refuses, and in a `WHERE` it **wrote**.
--
-- This fixture drives an array spelling 18.6 **accepts**, which is what keeps
-- "where Postgres accepts, any pglet error passes" a live assertion here instead of a
-- waiver: `'{arr_qt}'` is a well-formed one-element array literal there, whose element
-- names a relation, so 18.6 answers `{arr_qt}`. The swallowing reader threw the `[]` away
-- and looked the whole literal up as a *name*, which resolves against the relation
-- deliberately created as `"{arr_qt}"` — so before this rule pglet answered a row here,
-- and reverting it reddens this file. Its polarity — that the *scalar* spelling still
-- reads a brace-spelled literal as one identifier and answers the quoted relation name,
-- on both engines — is `regclass_quotes_the_relation_name.sql`'s `brace_literal` column.
--
-- The other spellings cannot be fixtures: pglet answers `42704` (the code its *column*
-- form already gave for the same type spelling) where 18.6 answers `22P02` for the array
-- value, `42601` for the modifier and `42846` for `1::regclass[]`, so a faithful
-- `@expect-error` over any of them would compare two different SQLSTATEs and be red.
-- They are pinned by SQLSTATE in `crates/pglet/tests/regclass_cast_target.rs`, with the
-- rows the `DELETE` no longer removes.
--
-- @no-profile-lock: the refusal is the cast target's type resolution, raised in the
-- parser before any plan exists, so neither profile reaches an index path.
-- @expect-error: 42704 — `regclass[]` is not a type pglet models
-- @setup
CREATE TABLE arr_qt (a INT);
CREATE TABLE "{arr_qt}" (a INT);

-- @test
SELECT ('{arr_qt}'::regclass[])::text
