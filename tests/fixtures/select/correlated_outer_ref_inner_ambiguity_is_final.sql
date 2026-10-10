-- **An ambiguity is final at the level that claims it.** `scanRTEForColumn` raises
-- `42702` at the innermost level whose relations claim a bare name and never walks
-- further out — so with `v` on both sides of the inner join this is `42702` on 18.6,
-- whatever the outer query carries. pglet read the inner resolver's `AmbiguousColumn` as
-- "not found here", escalated to the outer frame, and answered the row.
--
-- The escalation also **writes**: the `DELETE` spelling of this statement removed the
-- outer row, and `crates/pglet/tests/scope_relation_identity.rs` carries that assertion.
-- And it could hide as a *different* loud error rather than as an answer — give the outer
-- `v` a type the inner comparison rejects and the escalated binding surfaced as
-- `42883 text = integer`, which reads like an operand-typing bug and is this one.
--
-- The control that says the fixture is about escalation rather than about the join: drop
-- `iaf_d` from the inner FROM and the bare `v` is unambiguous and the query answers, on
-- both engines.
-- @setup
CREATE TABLE iaf_o (id INT, v INT);
CREATE TABLE iaf_c (id INT, v INT);
CREATE TABLE iaf_d (id INT, v INT);
INSERT INTO iaf_o VALUES (1, 5);
INSERT INTO iaf_c VALUES (1, 1);
INSERT INTO iaf_d VALUES (1, 1);

-- @expect-error: column reference "v" is ambiguous (42702)
-- @test
SELECT id FROM iaf_o WHERE EXISTS (SELECT 1 FROM iaf_c JOIN iaf_d ON iaf_c.id = iaf_d.id WHERE v = 5);
