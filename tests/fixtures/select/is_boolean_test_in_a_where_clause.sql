-- The predicate position, where the three-valued result is what changes the row
-- count: `b IS NOT TRUE` keeps the `false` row **and** the NULL one, where
-- `NOT b` keeps only the `false` row.
-- @setup
CREATE TABLE tb (id INT, b BOOL);
INSERT INTO tb VALUES (1, true), (2, false), (3, NULL);
-- @test
SELECT
  (SELECT count(*) FROM tb WHERE b IS NOT TRUE) AS not_true,
  (SELECT count(*) FROM tb WHERE NOT b) AS negated,
  (SELECT count(*) FROM tb WHERE b IS UNKNOWN) AS unknown;
