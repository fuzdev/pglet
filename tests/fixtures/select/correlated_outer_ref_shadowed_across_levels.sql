-- A qualified correlated reference binds to the frame the qualifier **names**, not to
-- the innermost frame that happens to carry the column. Both `sa` and `sb` have a `v`,
-- and the innermost subquery names the *outermost* one (`a4.v`).
--
-- This is the shape that separates binding by relation identity from a qualifier-blind
-- bare match: with no colliding name between the levels — which is how
-- `correlated_outer_ref_nested_two_levels.sql` is built — both answer the same column, so
-- neither that fixture nor a single-frame probe can tell them apart. Here a bare match
-- takes `sb`'s `v` (30), which `sc` does not hold. It stays a live discriminator after
-- the fallback's removal: restoring it reddens this fixture and no other in the family.
--
-- The control is the same statement spelled `b4.v`, which is `(0 rows)` on both engines
-- — the values are chosen so a mis-binding is a different row count rather than the
-- same one.
-- @setup
CREATE TABLE sa (id INT, v INT);
CREATE TABLE sb (id INT, v INT);
CREATE TABLE sc (k INT);
INSERT INTO sa VALUES (1, 20);
INSERT INTO sb VALUES (1, 30);
INSERT INTO sc VALUES (20);

-- @test
SELECT a4.id FROM sa a4
 WHERE EXISTS (SELECT 1 FROM sb b4
                WHERE b4.id = a4.id
                  AND EXISTS (SELECT 1 FROM sc WHERE sc.k = a4.v));
