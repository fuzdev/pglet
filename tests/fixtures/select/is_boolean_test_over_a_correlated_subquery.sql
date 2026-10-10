-- The operand is a **correlated** scalar subquery, so the node sits above the
-- correlation walkers as well as the evaluator — the readers that descend into a
-- sub-plan reach it through the shared child visitors rather than through an arm of
-- their own, and this is what shows that delegation still holds for the new node.
-- @setup
CREATE TABLE tb (id INT, b BOOL);
INSERT INTO tb VALUES (1, true), (2, false), (3, NULL);
-- @test
SELECT id FROM tb t1
WHERE (SELECT b FROM tb t2 WHERE t2.id = t1.id) IS TRUE
ORDER BY id;
