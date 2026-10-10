-- `information_schema.columns.datetime_precision` is a fact about the **type**, not
-- about whether a modifier was spelled: a bare `timestamp` reports `6` (its full
-- precision) while its `atttypmod` is `-1`, and a `date`, which carries no modifier at
-- all, reports `0`. Every non-temporal column is NULL.
--
-- An **array** column is NULL too — the same split `character_maximum_length` takes:
-- the catalog holds the *element's* modifier (a `timestamp(3)[]`'s `atttypmod` is `3`)
-- while this view describes the **column's** type, whose datetime precision no
-- `_timestamp` has.
-- @setup
CREATE TABLE tdp (a timestamp(3), b timestamptz(2), c time(1), d interval(4),
                  e timestamp, f date, g int, h varchar(5), i numeric(4,1),
                  j timestamp(3)[], k interval(3)[], l timestamp(9), m time, n interval);
-- @no-declared-types: `column_name` is `sql_identifier` (base `name`, OID 19) and
--   `data_type` is `character_data` (base `varchar`, 1043) on 18.6, where pglet declares
--   `text` (25) for both — measured by removing this waiver, and the only two columns
--   that diverge. `datetime_precision` itself agrees: 18.6's `cardinal_number` has base
--   `int4` (23), which is what pglet declares. Pre-existing catalog-type modelling.
-- @test
SELECT column_name, data_type, datetime_precision
FROM information_schema.columns
WHERE table_name = 'tdp'
ORDER BY ordinal_position;
