-- A **design lock**, not regression coverage: this passed before the coercion pass was
-- made qualifier-aware and passes after. What it pins is the deliberate split between
-- the gate's one-claimant rule and the coercion pass's claimants-agree rule — a future
-- strictening that collapsed the two would turn this legal query into a `42883`.
--
-- The positive case a strict one-claimant rule would break. `JOIN … USING (id)` merges
-- the two `id` columns into one, so a bare `id` is **not** ambiguous in Postgres and
-- `WHERE id = '1'` must still coerce the literal. The claimants necessarily agree on a
-- type (Postgres needs a common type to merge on), so the coercion pass resolves a
-- bare name whose claimants all agree — where the gate, which must not pre-empt
-- Postgres's own `42702` for a genuinely ambiguous name, requires exactly one.
-- @setup
CREATE TABLE jub_1 (id INT, a TEXT);
CREATE TABLE jub_2 (id INT, b TEXT);
INSERT INTO jub_1 VALUES (1, 'x'), (2, 'x2');
INSERT INTO jub_2 VALUES (1, 'y'), (2, 'y2');
-- @test
SELECT id, a, b FROM jub_1 JOIN jub_2 USING (id) WHERE id = '1';
