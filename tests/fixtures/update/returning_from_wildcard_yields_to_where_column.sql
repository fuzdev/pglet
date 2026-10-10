-- **Error precedence**, the `WHERE` member — see
-- `returning_from_wildcard_yields_to_missing_source.sql` for the class and its control.
--
-- `WHERE s.nosuchcol = 1` qualifies a column the *source* lacks: `42703` on both
-- engines. pglet's `WHERE` scope is **open** (a relation it could not enumerate may
-- still carry the name), so nothing rejects the reference at plan time and the `42703`
-- comes from eval — behind which a plan-time `RETURNING *` refusal used to sit.
--
-- It is also the one member where the two clauses read the *same* relation for opposite
-- verdicts: `RETURNING *` expands the source's real columns while `WHERE s.nosuchcol`
-- names one it does not have, so a namespace that had lost the source would redden here
-- by answering the wrong error, not by answering none.
-- @setup
CREATE TABLE wpw_t (id INT, v TEXT);
CREATE TABLE wpw_s (id INT, v TEXT, k INT);
INSERT INTO wpw_t VALUES (1, 'TARGET');
INSERT INTO wpw_s VALUES (1, 'SOURCE', 7);

-- @test
-- @expect-error: 42703 — the unknown WHERE column, on a statement whose RETURNING * resolves
UPDATE wpw_t SET id = 1 FROM wpw_s s WHERE s.nosuchcol = 1 RETURNING *;
