-- A container body holding nothing but a non-JSON whitespace character. The body is
-- trimmed by the object parser's own call, which is a different call site from the ones
-- the other whitespace fixtures reach -- narrowing three of the four and leaving this one
-- wide is green everywhere else and red here. The control `{ }` is in
-- `whitespace_json_four_accepted.sql`.
-- @expect-error: a form feed as an object body
-- @expect-message
-- @test
SELECT (E'{\x0c}')::jsonb;
