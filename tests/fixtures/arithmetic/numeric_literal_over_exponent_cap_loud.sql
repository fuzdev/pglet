-- 18.6's `set_var_from_str` gives up on the exponent's digits once the accumulated
-- exponent passes `PG_INT32_MAX / 2`, and what it raises there is `make_result`'s
-- `22003 value overflows numeric format` — an overflow, not a syntax error. Answered as
-- a syntax error it costs the code and a message that carries the whole literal:
-- `SELECT 1e<200 000 nines>` measured a 200 072-byte `ErrorResponse` that way, where
-- 18.6's is 97 bytes.
-- @test
-- @expect-error: exponent past the reader's cap (22003)
-- @expect-message
SELECT 1e2147483647;
