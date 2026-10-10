-- The catalog describes the **column**, whose type is the array: `data_type` is
-- `ARRAY` and `udt_name` the array's. `character_maximum_length` is NULL even though
-- the column has a cap, because no array type has a character length — the cap is the
-- element's, and it rides on `pg_attribute.atttypmod` instead.
-- @setup
CREATE TABLE tmshape (a varchar(3)[], b numeric(5,2)[]);
-- @no-declared-types: the view's own column types diverge — 18.6 sends `name` (19) for
--   `column_name` / `udt_name` and `information_schema.character_data` for `data_type`,
--   where pglet sends `text` (25). Pre-existing catalog-type modelling, not this
--   fixture's subject; the row values are what it asserts.
-- @test
SELECT column_name, data_type, udt_name, character_maximum_length, numeric_precision, numeric_scale
FROM information_schema.columns
WHERE table_name = 'tmshape'
ORDER BY ordinal_position;
