-- A non-finite `float8` cast to `text` is spelled `Infinity` / `-Infinity`, as `float8out`
-- spells it — the text a `date_part` of an infinite date or timestamp becomes.
-- @test
SELECT date_part('year', date 'infinity')::text AS a, date_part('epoch', timestamptz '-infinity')::varchar AS b,
       'Infinity'::float8::text AS c, '-Infinity'::float4::text AS d, 'NaN'::float8::text AS e;
