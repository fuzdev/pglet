-- A table-level CHECK spanning two columns (the spine fact-store shape:
-- at least one of `bytes` / `external_url` must be non-null).
-- @setup
CREATE TABLE factlike (
  id INT,
  bytes TEXT,
  external_url TEXT,
  CHECK (bytes IS NOT NULL OR external_url IS NOT NULL)
);

-- @test
-- @expect-error: both columns null violates the multi-column CHECK
INSERT INTO factlike (id) VALUES (1);
