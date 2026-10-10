-- The same gate, at a key: one decoder serves both positions. This is the input whose
-- acceptance made row 135's dump of a raw-tab key load into pglet and not into 18.6.
-- @expect-error: a raw tab inside a JSON object key
-- @expect-message
-- @test
SELECT (E'{"a\tb": 1}')::jsonb;
