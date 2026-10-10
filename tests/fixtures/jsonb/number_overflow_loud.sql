-- A syntactically-valid JSON number that overflows the numeric format fails loud
-- with `22003` ("value overflows numeric format"), NOT the generic `22P02`.
-- Postgres runs each jsonb number token through `numeric_in`, so `1e200000`
-- overflows exactly as the direct `::numeric` cast does. A jsonb number arm that
-- swallows the specific error answers `22P02` instead.
-- @expect-error: jsonb number overflows numeric format (22003)
-- @expect-message
-- @test
SELECT '1e200000'::jsonb;
