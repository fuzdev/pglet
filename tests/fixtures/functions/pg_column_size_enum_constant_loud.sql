-- An enum member literal reaches pglet's planner as the same cast to `text` a string
-- written as `'ok'::text` does (`EDGES.md` row 104), so a string literal cast to `text`
-- is refused `0A000` rather than sized as text: 18.6 stores an enum as its four-byte OID
-- (`pg_column_size('ok'::mood)` is 4 there, and was 6 here, the label's size). The
-- refusal costs `'ok'::text` its answer too. Documentary where 18.6 answers.
-- @setup
CREATE TYPE pce_mood AS ENUM ('ok');
-- @test
-- @expect-error: pg_column_size of an enum member literal
SELECT pg_column_size('ok'::pce_mood);
