-- A redundantly parenthesized `DEFAULT` is the bare form. 18.6 strips the parens in
-- parse analysis, so `(5)`, `(-7)`, `('x')`, `(true)`, `(NULL)`, `(now())` and
-- `(nextval('s'))` each behave — and render in
-- `information_schema.columns.column_default` — exactly as their bare spellings do
-- (measured: `DEFAULT (5)` renders `5`, `DEFAULT (-7)` renders `'-7'::integer`,
-- identically to the unparenthesized forms).
--
-- pglet's DEFAULT classifier is token-level and used to decline anything starting
-- with `(`, so every one of these was `0A000`. They now reduce onto the same closed
-- `ColumnDefault` arms the bare forms take, which matters beyond the over-rejection:
-- a parenthesized `nextval` that fell through to a general expression would lose the
-- sequence's ownership edge and its dump hoisting along with it.
-- @setup
CREATE SEQUENCE dpf_seq;
CREATE TABLE dpf (
  k INT,
  a INT DEFAULT (5),
  b INT DEFAULT ((5)),
  c INT DEFAULT (-7),
  d NUMERIC DEFAULT (+1.5),
  e TEXT DEFAULT ('x'),
  f BOOL DEFAULT (true),
  g INT DEFAULT (NULL),
  h INT DEFAULT (nextval('dpf_seq'))
);
INSERT INTO dpf (k) VALUES (1);
INSERT INTO dpf (k) VALUES (2);
-- @test
SELECT k, a, b, c, d, e, f, g, h FROM dpf ORDER BY k;
