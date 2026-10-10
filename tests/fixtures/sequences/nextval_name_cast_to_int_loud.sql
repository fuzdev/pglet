-- The name argument is reduced to a `String` and its expression discarded, so the cast
-- has to be coerced before it is dropped or its error is lost with it: pglet looked
-- through `::int` to the literal and **advanced the sequence**, where Postgres
-- evaluates `'x'::int` and raises `22P02`.
-- @setup
CREATE SEQUENCE nncil_seq;
-- @test
SELECT nextval('nncil_seq'::int);
-- @expect-error: the name argument's own cast is unparseable (22P02)
