-- Which error wins, one scale down from `update/returning_error_precedes_set_error.sql`:
-- inside a single `RETURNING` item, the **leaf's** ambiguity is reported ahead of the
-- operand gate on the node above it.
--
-- `v` is claimed by the target and by the `FROM` source, and `v LIKE 5` is `text ~~
-- integer` — two independent errors in one expression. Postgres resolves a leaf when it
-- reaches it, so it reports the leaf: `42702`. pglet reported the operator's `42883`,
-- because the ambiguity was decided nowhere and the bare name simply resolved to the
-- target's `v`.
--
-- The two clauses raise different SQLSTATEs on purpose, and both engines reject, so the
-- SQLSTATE is compared: a fix that put the ambiguity check in a pass of its own — before
-- or after the operand walk — scores the wrong one of the pair here, which no
-- single-error fixture can see. (Its mirror is
-- `delete/returning_operand_gate_precedes_binding.sql`, where the operand error is the
-- *left* one and wins.)
-- @setup
CREATE TABLE rbo_t (id INT, v TEXT);
CREATE TABLE rbo_s (id INT, v TEXT, k INT);
INSERT INTO rbo_t VALUES (1, 'TARGET');
INSERT INTO rbo_s VALUES (1, 'SOURCE', 7);

-- @test
-- @expect-error: 42702 — the ambiguous leaf beats the ill-typed operator above it
UPDATE rbo_t SET id = 1 FROM rbo_s s WHERE s.id = rbo_t.id RETURNING v LIKE 5;
