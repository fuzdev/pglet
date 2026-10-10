-- Postgres `FigureColname` names an unaliased explicit CAST of a **nameless**
-- (literal) operand after the target type's canonical internal name — `1::int4`
-- → int4, `'a'::varchar(5)` → varchar, `1.5::float8` → float8, `9::numeric(10,2)`
-- → numeric — not the generic `?column?`. A cast of a **named** column instead
-- keeps that column's name (`x::int8` → x), looking through the cast.
-- @setup
CREATE TABLE cast_names (x INT);
INSERT INTO cast_names VALUES (7);

-- @test
SELECT
  x::int8,
  100::int4,
  'a'::varchar(5),
  5::text,
  1.5::float8,
  9::numeric(10,2),
  true::bool
FROM cast_names;
