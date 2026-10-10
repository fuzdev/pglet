-- The truncation's other side: 60 dashes then `'utf8'` is cut to 60 dashes and `'utf'`, a name
-- 18.6 does not know — its own `22023 invalid value for parameter "client_encoding"`.
-- @test
-- @expect-error: invalid value for parameter "client_encoding"
SET client_encoding = '------------------------------------------------------------utf8';
