-- **Error precedence**, the `SET`-value member — see
-- `returning_from_wildcard_yields_to_missing_source.sql` for the class and its control.
--
-- `SET id = 'notanint'` on an `INT` column is `22P02` on both engines. It is the member
-- that falsifies the "the clause order holds on the type axis" reading of this area: the
-- divergence a plan-time `RETURNING *` refusal introduced was a **type** error losing to
-- a name-shaped gate, not only a name one.
--
-- **The row must match.** pglet coerces a `SET` value per row, so with no matching row
-- it writes nothing and reports `UPDATE 0` where 18.6 raises `22P02` at parse analysis
-- (`UPDATE wpv_t SET id = 'notanint' WHERE id = 999` diverges the same way, with no
-- `FROM` and no `RETURNING` in sight). That divergence is not this fixture's subject,
-- so the seed and the join predicate are chosen to match — write them otherwise and
-- this file stops testing precedence and starts testing that unrelated gap.
-- @setup
CREATE TABLE wpv_t (id INT, v TEXT);
CREATE TABLE wpv_s (id INT, v TEXT, k INT);
INSERT INTO wpv_t VALUES (1, 'TARGET');
INSERT INTO wpv_s VALUES (1, 'SOURCE', 7);

-- @test
-- @expect-error: 22P02 — the ill-typed SET value, on a statement whose RETURNING * resolves
UPDATE wpv_t SET id = 'notanint' FROM wpv_s s WHERE s.id = wpv_t.id RETURNING *;
