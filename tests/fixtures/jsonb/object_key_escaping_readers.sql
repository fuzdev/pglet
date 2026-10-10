-- Every surface that renders a jsonb value as JSON text reads one predicate, so the key's
-- spelling is the same at each: the `::text` and `::varchar` casts, the value at the wire
-- (rendered by the text encoder), and `->>` / `#>>` over a *container*, which fall back to
-- the same renderer. The scalar `->>` beside them is the control that must NOT escape --
-- it returns the string itself, not JSON text.
-- @setup
CREATE TABLE jk (id int, v jsonb);
INSERT INTO jk VALUES (1, '{"o": {"a\tb": 1}, "s": "a\tb"}');
-- @test
SELECT (v -> 'o')::text AS cast_text,
       (v -> 'o')::varchar AS cast_varchar,
       v -> 'o' AS at_the_wire,
       v ->> 'o' AS arrow2_container,
       v #>> '{o}' AS hash_container,
       v ->> 's' AS arrow2_scalar
FROM jk;
