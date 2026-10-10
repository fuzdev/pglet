-- The spine's `fact_ref(source_hash, target_hash)` composite-PK shape: tuples
-- sharing one column coexist, and the mirror tuple (swapped columns) is distinct.
-- @setup
CREATE TABLE fact_ref (source_hash TEXT NOT NULL, target_hash TEXT NOT NULL, PRIMARY KEY (source_hash, target_hash));
INSERT INTO fact_ref VALUES
  ('blake3:a', 'blake3:b'),
  ('blake3:a', 'blake3:c'),
  ('blake3:b', 'blake3:b'),
  ('blake3:b', 'blake3:a');

-- @test
SELECT source_hash, target_hash FROM fact_ref ORDER BY source_hash, target_hash;
