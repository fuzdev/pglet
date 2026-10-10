-- The **aggregate** walk is a second expression evaluator, so every rule here has
-- a second home in it. A `CASE` inside an aggregate argument counted nothing per
-- group — a silently wrong aggregate result on a query 18.6 answers.
-- @setup
CREATE TABLE bcag (a INT, b TEXT);
INSERT INTO bcag VALUES (1, 'x'), (2, 'y');
-- @test
SELECT b, count(CASE WHEN 't' THEN 1 END) FROM bcag GROUP BY b ORDER BY b;
