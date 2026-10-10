-- The `HAVING` call site of the aggregate evaluator, which is a **second** entry to
-- the same walk as `is_boolean_test_over_an_aggregate.sql`'s projection and can fail
-- on its own: a group that the test rejects must not reach the output.
-- @setup
CREATE TABLE tb (id INT, i INT);
INSERT INTO tb VALUES (1, 10), (1, 20), (2, 30), (3, NULL);
-- @test
SELECT id, count(*) AS n
FROM tb
GROUP BY id
HAVING (count(*) > 1) IS TRUE OR (sum(i) IS NULL) IS TRUE
ORDER BY id;
