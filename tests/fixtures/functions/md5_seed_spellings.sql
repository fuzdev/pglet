-- The seeding spellings test SQL reaches for: a prefix of a digest, a digest of a
-- cast, and a digest compared as text.
-- @setup
CREATE TABLE ms (id INT);
INSERT INTO ms VALUES (1), (2), (3);
-- @test
SELECT id, left(md5(id::text), 8) AS short, md5(id::text) = md5(CAST(id AS text)) AS same,
  substr(md5('x' || id), 1, 4) AS pre
FROM ms ORDER BY id;
