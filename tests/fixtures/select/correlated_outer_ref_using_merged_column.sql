-- **A false-rejection control — do not trim it.** The outer-chain claimant count is
-- exact-match only, and this is the shape that says why: a `JOIN … USING (id)` frame
-- carries the merged bare `id` ahead of the raw `a.id` / `b.id`, so exactly one exact
-- claimant answers a correlated bare `id`. A count that also took suffix matches would
-- see three and raise a spurious `42702` here, and a count that took the *first* match
-- would be right by luck.
--
-- The rule that has to hold beside it is the ambiguity one, whose fixtures are
-- `select/correlated_outer_ref_duplicate_output_name_{qualified,bare}.sql`. Nesting a
-- second `USING (id)` merge keeps a single bare claimant too, measured on both engines.
-- @setup
CREATE TABLE ucm_a (id INT, x INT);
CREATE TABLE ucm_b (id INT, y INT);
CREATE TABLE ucm_z (k INT);
INSERT INTO ucm_a VALUES (1, 11), (2, 22);
INSERT INTO ucm_b VALUES (1, 33), (2, 44);
INSERT INTO ucm_z VALUES (1);

-- @test
SELECT x FROM ucm_a JOIN ucm_b USING (id) WHERE EXISTS (SELECT 1 FROM ucm_z WHERE ucm_z.k = id) ORDER BY x;
