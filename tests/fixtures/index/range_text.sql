-- Range over a TEXT indexed column: `WHERE name >= 'c' AND name < 'g'` plans as
-- an IndexRangeScan, bounded by the index's order-preserving text key. The
-- half-open lexicographic window must match Postgres.
-- @profile-lock
-- @setup
CREATE TABLE rng_text (id INT, name TEXT NOT NULL);
INSERT INTO rng_text VALUES
  (1, 'apple'),
  (2, 'cherry'),
  (3, 'date'),
  (4, 'fig'),
  (5, 'grape'),
  (6, 'kiwi');
CREATE INDEX rng_text_name ON rng_text(name);
-- @test
SELECT id, name FROM rng_text WHERE name >= 'c' AND name < 'g' ORDER BY name, id;
