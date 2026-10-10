-- The accept side of the declaration-only operand gate at the **negate** node, over an
-- aggregate (`- max(qty)`, the gate that makes `- max(conrelid)` loud) and over the
-- aggregate's argument. `- oid` is `42883` on 18.6 and every line here is an ordinary
-- integer, which 18.6 answers.
--
-- The triple negation pins that a negate node hands its operand's type up: each `-`
-- reads the type the one below it derived. The negate-over-a-**scalar-subquery** accept
-- is pinned by `oid_gate_accept_polarity_scalar_subquery.sql` instead: a scalar subquery
-- anywhere in a grouping query is pglet's own `0A000` (`EDGES.md` row 89), so it cannot
-- ride beside an aggregate here.
-- @setup
CREATE TABLE oid_neg (qty INT);
INSERT INTO oid_neg VALUES (7), (9);

-- @test
SELECT -max(qty) AS neg_aggregate,
       -(-(-min(qty))) AS neg_thrice,
       -max(qty) + 1 AS neg_then_shifted
FROM oid_neg
