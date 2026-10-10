-- @setup
CREATE TABLE trunc_a (id INT);
CREATE TABLE trunc_b (id INT);
INSERT INTO trunc_a VALUES (1), (2);
INSERT INTO trunc_b VALUES (10);
TRUNCATE trunc_a, trunc_b;

-- @test
SELECT
  (SELECT count(*) FROM trunc_a) AS a,
  (SELECT count(*) FROM trunc_b) AS b;
