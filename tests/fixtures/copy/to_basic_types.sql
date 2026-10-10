-- COPY ... TO STDOUT of the spine-relevant types: the emitted bytes must match
-- Postgres exactly (column-type-aware text encoder: timestamptz carries +00, bytea
-- is \x hex, NULL is \N). jsonb keys are single/canonical so pglet's
-- insertion-order rendering equals Postgres's canonicalized order.
-- @setup
CREATE TABLE t (
  id int,
  name text,
  active bool,
  tags text[],
  uid uuid,
  meta jsonb,
  ts timestamptz,
  payload bytea
);
INSERT INTO t VALUES
  (1, 'alice', true, ARRAY['a','b'], '11111111-1111-1111-1111-111111111111', '{"k": 1}', '2024-06-15 12:30:45.123+00', '\xdeadbeef'),
  (2, 'bob', false, ARRAY['c','d e'], '22222222-2222-2222-2222-222222222222', '{"k": 2}', '2024-01-02 03:04:05+00', '\x00ff'),
  (3, NULL, NULL, NULL, NULL, NULL, NULL, NULL);
-- @copy-to
COPY t TO STDOUT;
