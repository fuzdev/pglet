-- The `UPDATE … SET` spelling of the assignment-cast gate. Same cause as the `INSERT`
-- one — `coerce_to_type` wrapped the right-hand side in an explicit cast to the column
-- type, so `int4 ↔ bool` (a `pg_cast` `'e'` pair) was performed on the write and this
-- **stored `true`**.
--
-- The `WHERE` matches **no row** on purpose: an eval-time refusal is not raised at all
-- when nothing is updated, so a statement 18.6 refuses would otherwise report
-- `UPDATE 0`. That is the row-count-dependence this engine keeps closing.
-- @setup
CREATE TABLE acup (id INT, b BOOL);
INSERT INTO acup VALUES (1, FALSE);

-- @expect-error: column "b" is of type boolean but expression is of type integer (42804)
-- @test
UPDATE acup SET b = 1 WHERE 1 = 0;
