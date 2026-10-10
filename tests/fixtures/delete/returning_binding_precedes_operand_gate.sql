-- Error precedence between the two plan-time checks that now share one walk. Postgres
-- resolves a `RETURNING` item's leaves as it reaches them, so the *leftmost* fault wins:
-- the bad column here sits left of the bad operator, and `42703` is the answer. Its
-- mirror is `delete/returning_operand_gate_precedes_binding.sql`, where the operator sits
-- left and `42883` is the answer. **Neither ordering of one whole-tree binding pass
-- against one whole-tree operand pass yields both** — a binding pass placed before the
-- gate makes the mirror `42703`, one placed after makes this fixture `42883` — which is
-- why the binding check is a per-node arm of `check_expr_node_semantics` rather than a
-- pass of its own. Trimming either half of the pair loses the discrimination.
-- @setup
CREATE TABLE rbp_t (id INT, b BIGINT);
INSERT INTO rbp_t VALUES (1, 111);

-- @test
-- @expect-error: the leftmost fault is the unbindable column (42703), not the operator (42883)
DELETE FROM rbp_t WHERE id = 999 RETURNING bogus > 0 AND b LIKE 'x';
