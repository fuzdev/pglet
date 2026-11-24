-- query_fact_holder_unheld (fuz's `fuz_cell_actions` cell ref admission): the hashes an
-- actor does not hold, in input order — `unnest($2::text[]) WITH ORDINALITY AS h(hash, ord)`,
-- the alias list naming both the element and the ordinality column, a correlated
-- `NOT EXISTS` reading `h.hash`, and `ORDER BY h.ord`. The input order is not the hashes'
-- own order, so a sort by anything but the ordinality answers differently.
-- @setup
CREATE TABLE fact_holder (
  actor_id uuid NOT NULL,
  hash text NOT NULL,
  PRIMARY KEY (actor_id, hash)
);
INSERT INTO fact_holder (actor_id, hash) VALUES
  ('11111111-1111-1111-1111-111111111111', 'blake3:b'),
  ('22222222-2222-2222-2222-222222222222', 'blake3:c');
-- @param $1 uuid 11111111-1111-1111-1111-111111111111
-- @param $2 text[] {blake3:d,blake3:b,blake3:c,blake3:a}
-- @test
SELECT h.hash FROM unnest($2::text[]) WITH ORDINALITY AS h(hash, ord)
WHERE NOT EXISTS (
  SELECT 1 FROM fact_holder fh WHERE fh.actor_id = $1 AND fh.hash = h.hash
)
ORDER BY h.ord;
