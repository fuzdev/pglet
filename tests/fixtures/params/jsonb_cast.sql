-- `$1::text::jsonb` — the spine sends JSON as a text parameter and casts it to
-- jsonb, then reads a field.
-- @param $1 text {"kind":"doc","n":42}
-- @test
SELECT ($1::text::jsonb ->> 'kind') AS kind;
