-- **A control — do not trim it.** A relation whose *own name* carries a dot. It answers
-- on both engines before and after, and no mutation of the pair derivation reddens it —
-- what it pins is the shape a **name-splitting** derivation would get wrong: splitting
-- `"a.b".x`'s synthesized spelling at the first `.` yields the pair `("a", "b.x")`. The
-- pairs come from the plan, so the alias stays one name; the fixture exists so that a
-- later "just split the name" simplification cannot land silently.
-- @setup
CREATE TABLE jdar_a (id INT, x INT);
CREATE TABLE jdar_b (id INT, y INT);
INSERT INTO jdar_a VALUES (1, 11), (2, 22);
INSERT INTO jdar_b VALUES (1, 33), (2, 44);

-- @test
SELECT "a.b".x FROM jdar_a AS "a.b" JOIN jdar_b b ON b.id = "a.b".id ORDER BY "a.b".x;
