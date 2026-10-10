-- Table-level `FOREIGN KEY (col) REFERENCES parent(col) ON DELETE CASCADE`
-- (the spine's `fact_ref.source_hash -> fact(hash)` shape: a single-column FK on
-- a table whose own PK is composite). Deleting the parent cascades to the child.
-- @setup
CREATE TABLE fact (hash TEXT PRIMARY KEY, size BIGINT NOT NULL);
CREATE TABLE fact_ref (
  source_hash TEXT NOT NULL,
  target_hash TEXT NOT NULL,
  PRIMARY KEY (source_hash, target_hash),
  FOREIGN KEY (source_hash) REFERENCES fact(hash) ON DELETE CASCADE
);
INSERT INTO fact VALUES ('blake3:src', 1), ('blake3:other', 2);
INSERT INTO fact_ref VALUES
  ('blake3:src', 'blake3:t1'),
  ('blake3:src', 'blake3:t2'),
  ('blake3:other', 'blake3:t9');
DELETE FROM fact WHERE hash = 'blake3:src';

-- @test
-- The src refs cascade away; the other fact's ref survives.
SELECT source_hash, target_hash FROM fact_ref ORDER BY source_hash, target_hash;
