-- **A false-rejection control — do not trim it.** `transformDistinctOnClause` groups on
-- the **ON expressions only**, so a `DISTINCT ON` over a source column claims no
-- target-list entry and the `'t'` here stays Postgres's `unknown`. Accepted on 18.6 —
-- unlike its plain-`DISTINCT` neighbour, which groups on every entry and refuses
-- (`assignment_cast_select_distinct_resolves_the_unknown_loud.sql`). One word apart, and
-- opposite verdicts.
--
-- What it says about the implementation is narrower than it once claimed: the
-- `Plan::DistinctOn` node passes through, which the mutation that deletes that arm
-- confirms (it reddens this fixture). It says nothing about how a *claiming* ON key is
-- recorded — this key claims none — so the fixture for that is
-- `assignment_cast_select_distinct_on_ordinal_resolves_the_unknown.sql`, whose claim is
-- read off the written ordinal at the lowering, before `resolve_output_position`
-- canonicalizes it away.
-- @setup
CREATE TABLE acdo (id INT, b BOOL);
CREATE TABLE acdosrc (n INT);
INSERT INTO acdosrc VALUES (1), (1), (2);
-- @test
INSERT INTO acdo (b) SELECT DISTINCT ON (n) 't' FROM acdosrc RETURNING b;
