-- The residual beside it: read through a **named** entry rather than a wildcard, the
-- derived table's own column type is the one `output_columns` guessed (`text`, for a
-- reference its level cannot resolve), so the branch set is refused where 18.6 answers
-- the row. Loud, and pglet's alone — the recorded shape is Postgres's success.
-- @setup
CREATE TABLE ssdn (n NUMERIC);
INSERT INTO ssdn VALUES (1.25);
-- @test
SELECT coalesce((SELECT z FROM (SELECT o.n AS z) w), 2) AS v FROM ssdn o;
-- @expect-error: COALESCE types text and integer cannot be matched — pglet-only
