-- information_schema.columns.column_default for the new functions, as 18.6 prints it.
-- @setup
CREATE TABLE cdt (id INT,
  l TEXT DEFAULT left('abcdef', 2),
  r TEXT DEFAULT "right"('abcdef', 2),
  s TEXT DEFAULT substr('abcdef', 2),
  o INT DEFAULT octet_length('abc'),
  m TEXT DEFAULT md5('a'),
  md INT DEFAULT mod(7, 3),
  rn DOUBLE PRECISION DEFAULT random());
-- @no-declared-types: 18.6 sends `name` (19) for `column_name` and
--   `information_schema.character_data` for `column_default`, where pglet sends `text`
--   (25) — pre-existing catalog-type modelling, not this fixture's subject.
-- @test
SELECT column_name, column_default FROM information_schema.columns
WHERE table_name = 'cdt' AND column_default IS NOT NULL ORDER BY ordinal_position;
