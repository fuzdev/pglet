-- `information_schema.columns.column_default` is the third reader of the one Catalog
-- rendering, and a column `DEFAULT` is the only one of them a `CREATE TABLE` can reach
-- without a constraint. The modifier is the element's, so it renders inside the `[]`.
-- @setup
CREATE TABLE tmdef (a varchar(3)[] DEFAULT '{ab}'::varchar(3)[],
  b numeric(4,1)[] DEFAULT '{9.9}'::numeric(4,1)[]);
-- @no-declared-types: 18.6 sends `name` (19) for `column_name` and
--   `information_schema.character_data` for `column_default`, where pglet sends `text`
--   (25) — pre-existing catalog-type modelling, not this fixture's subject.
-- @test
SELECT column_name, column_default
FROM information_schema.columns
WHERE table_name = 'tmdef'
ORDER BY ordinal_position;
