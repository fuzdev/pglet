-- **The row-count boundary the executor's rule could not reach.** An earlier repair made the
-- correlated outer-scope chain refuse a qualifier no frame owns, but that gate is at
-- eval: with the inner relation **empty** the reference is never evaluated and this was
-- `DELETE 0`, silently, where 18.6 raises `42P01` in parse analysis.
--
-- Closing it took the chain-completeness signal rather than a new rule: a nested level's
-- scope at its own lowering is its FROM alone — the enclosing query does not exist yet —
-- so the gate can only fire on `resolve_correlated_subplans`' re-run, where the chain is
-- whole. Its populated sibling is `delete/correlated_outer_ref_bogus_qualifier_loud.sql`,
-- green under either gate, which is why this one seeds nothing into `cbe_u`.
-- @setup
CREATE TABLE cbe_hw (id INT, v TEXT);
CREATE TABLE cbe_u (yv TEXT);
INSERT INTO cbe_hw VALUES (1, 'hit');

-- @expect-error: missing FROM-clause entry for table "zzz" (42P01)
-- @test
DELETE FROM cbe_hw WHERE EXISTS (SELECT 1 FROM cbe_u WHERE cbe_u.yv = zzz.v);
