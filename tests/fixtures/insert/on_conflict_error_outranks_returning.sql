-- Which clause reports when an `INSERT`'s `ON CONFLICT` action **and** its `RETURNING`
-- are both unbindable. `analyze.c` resolves an INSERT's row source, then `ON CONFLICT`,
-- then `RETURNING`, so the action's error wins — and `correlation::walk_level`'s
-- `Plan::Insert` arm must visit the two clauses in that order.
--
-- **The two clauses must raise different SQLSTATEs, or this fixture pins nothing.** The
-- DSL compares the code, never the message, so the obvious shape — an unbindable bare key
-- in each clause — is `42703` whichever clause reports and stays green with the order
-- inverted (measured: it did). Here the action carries a bogus *qualifier* (`42P01`, the
-- other arm of `check_binding`) and `RETURNING` an unbindable bare *key* (`42703`), so
-- the reported code names the clause. Its mirror
-- (`insert/returning_error_loses_to_on_conflict.sql`) swaps which clause carries which,
-- and 18.6 answers the action's code in both — one polarity alone would be satisfied by
-- an engine that simply always reports `42P01`.
--
-- `oco_src` stays empty and the row does not conflict, for the reasons the sibling
-- fixtures in this directory state: either would let the eval-time resolver raise
-- instead, and it reaches the two clauses in the other order.
-- @profile-lock
-- @setup
CREATE TABLE oco_target (id INT PRIMARY KEY, v INT);
CREATE TABLE oco_src (b INT);
INSERT INTO oco_target VALUES (1, 1);

-- @expect-error: missing FROM-clause entry for table "zzzrel" (42P01) — the action, not RETURNING's 42703
-- @test
INSERT INTO oco_target VALUES (3, 3)
  ON CONFLICT (id) DO UPDATE SET v = (SELECT b FROM oco_src WHERE zzzrel.q = 1)
  RETURNING (SELECT b FROM oco_src ORDER BY bbb);
