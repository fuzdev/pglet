-- Two FROM relations sharing a column name, with **both** sides' `x` projected. The
-- extended protocol's `Describe` resolves each output column's type against the join's
-- flattened input columns, whose names are bare — so looking a projected `jsp_b.x` up
-- by name alone merged the two sides and every reference took the **first** `x`'s
-- descriptor. `RowDescription` then declared `integer` for the `text` column's bytes
-- while the executor (which resolves the reference correctly) sent the right value: a
-- binary client decodes garbage or fails. Reversing the projection order moves the
-- damage to the other column rather than fixing it — it is a bare-name merge, not an
-- off-by-one — so the wrong-typed reference is projected **first** here.
--
-- The alias arm shares the lookup and was equally wrong, so `bx` covers it; `ax` holds
-- the side the merge happened to answer correctly. Output names are kept distinct
-- because the harness keys its row JSON by column name.
-- @setup
CREATE TABLE jsp_a (x INT, label TEXT);
CREATE TABLE jsp_b (x TEXT);
INSERT INTO jsp_a VALUES (1, 'only');
INSERT INTO jsp_b VALUES ('z');
-- @test
SELECT jsp_b.x, jsp_b.x AS bx, jsp_a.x AS ax FROM jsp_a JOIN jsp_b ON true;
