-- nullif(a, b): NULL when a = b, else a; a NULL argument returns a (without
-- comparing), so nullif(NULL, 5) and nullif(5, 5) are both NULL.
-- @setup
CREATE TABLE nf (id INT, a INT, b INT);
INSERT INTO nf VALUES (1, 5, 5), (2, 5, 6), (3, NULL, 5), (4, 5, NULL);
-- @test
SELECT id,
  nullif(a, b) AS nab,
  nullif(a, 5) AS na5
FROM nf ORDER BY id;
