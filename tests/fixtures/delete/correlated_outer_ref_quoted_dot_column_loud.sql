-- A qualified correlated reference may resolve only against an outer frame whose
-- relation identity **is** the qualifier. `q1`'s column is quoted into existence as
-- `"a.b"`, which is byte-identical to the `relation.column` string a *multi*-relation
-- outer frame addresses its columns by — so an ungated dotted match let a frame that
-- knows perfectly well it is `q1` answer for a relation named `a`. This statement then
-- **deleted a row** where 18.6 raises `42P01` and both rows survive; the destruction is
-- the point, and it is why the refusal is pinned on the `DELETE` spelling.
--
-- Controls measured against 18.6, all agreeing on both engines and each ruling out a
-- different cause: `SELECT a.b FROM q1` is `42P01` on both (the top-level gate was never
-- the missing one — this is outer-frame-only); `q2.yv = zzz.nosuch` is `42P01` on both
-- (so it was not the qualifier-blind bare fallback that refused it, since that searched
-- for a bare `nosuch` and found none); and `q2.yv = "a.b"` — the column under its real, quoted name —
-- answers on both.
--
-- The accept polarity of the same tier lives in
-- `update/returning_from_source_correlated_subquery.sql` and
-- `select/qualifier_valid_join_correlated.sql`, whose outer frames genuinely have no
-- single relation and so are still addressed dotted — both go red if the dotted tier is
-- removed rather than gated.
--
-- `q1` deliberately has **no** bare `b`, and this fixture pins the *dotted tier* rather
-- than the class: while a qualifier no frame owned fell through to a bare match, adding an
-- ordinary `b` beside the quoted column made this same statement destroy a row again.
-- Both halves are closed now — the chain declines a qualifier no frame owns — and the
-- escape route has its own fixture, `delete/correlated_outer_ref_bogus_qualifier_loud.sql`,
-- whose `zzz` needs no quoted column anywhere. Keep this one anyway: it is the only
-- fixture whose refusal turns on the frame *having* an identity, so it is what separates
-- the gate from a rule that simply refuses every dotted spelling.
-- @setup
CREATE TABLE q1 (id INT, "a.b" TEXT);
CREATE TABLE q2 (yv TEXT);
INSERT INTO q1 VALUES (1, 'hit'), (2, 'miss');
INSERT INTO q2 VALUES ('hit');

-- @expect-error: missing FROM-clause entry for table "a" (42P01)
-- @test
DELETE FROM q1 WHERE EXISTS (SELECT 1 FROM q2 WHERE q2.yv = a.b);
