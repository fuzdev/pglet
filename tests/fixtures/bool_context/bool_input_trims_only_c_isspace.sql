-- The trim set is C `isspace` in the `C` locale, not Rust's `str::trim`: a
-- no-break space is not whitespace to `boolin`, and trimming it accepted a value
-- 18.6 refuses.
-- @expect-error: invalid input syntax for type boolean
-- @test
SELECT (U&'\00A0' || 't')::bool;
