-- Every column takes its own default through the row-less source, and this is the
-- first route by which `DEFAULT VALUES` reaches a stored `ColumnDefault::Expr`: `n`
-- keeps a live `1 + 1` and `k` draws `nextval('dv_mix_s') + 10` **per row**, so the
-- two inserts answer 11 and 12 rather than one folded constant. `plain` carries no
-- default at all and is NULL beside them, which is what keeps the fixture from
-- passing on an engine that fills every column from the sequence family alone.

-- @setup
CREATE SEQUENCE dv_mix_s;
CREATE TABLE dv_mix (
  id SERIAL,
  n INT DEFAULT 1 + 1,
  t TEXT DEFAULT 'hi',
  d DATE DEFAULT '2020-01-01'::date,
  k INT DEFAULT nextval('dv_mix_s') + 10,
  plain INT
);
INSERT INTO dv_mix DEFAULT VALUES;
INSERT INTO dv_mix DEFAULT VALUES;

-- @test
SELECT id, n, t, d, k, plain FROM dv_mix ORDER BY id;
