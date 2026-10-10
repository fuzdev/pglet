-- **What namespace a leaf resolves against**, which is the half a fixture over `bogus`
-- cannot show: the set-op's *output* names and nothing else. 18.6's
-- `transformSetOperationStmt` makes the output columns visible as a join RTE, so an arm's
-- own column name that the projection renames is not in scope — the DETAIL there says it
-- in as many words (`There is a column named "x" in table "*SELECT* 2", but it cannot be
-- referenced from this part of the query`).
--
-- The left arm labels its column `lbl`, so `lbl` is the only name the clause can see, and
-- routing the leaf to the *arms* instead would answer this. The arms are **empty** for the
-- same reason the bare `bogus+1` fixture's are: seed a row into each and the eval-time
-- resolver raises the identical `42703` on its own, so a seeded spelling is green with the
-- plan-time gate reverted and guards nothing (measured as a mutation).
--
-- The accept control that identifies the choice is not fixturable: `ORDER BY lbl + 0` sorts
-- here and is 18.6's `0A000` for being an expression at all, the leniency this engine keeps
-- (`a_set_op_computed_sort_key_over_bound_leaves_still_sorts`).
-- @setup
CREATE TABLE sopca (x INT);
CREATE TABLE sopcb (x INT);

-- @expect-error: column "x" does not exist — an arm's name is not an output name (42703)
-- @test
SELECT x AS lbl FROM sopca UNION SELECT x FROM sopcb ORDER BY x + 0;
