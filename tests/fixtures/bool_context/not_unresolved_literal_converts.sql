-- `NOT` over an unresolved literal converts it and negates; it was `0A000` here.
-- @test
SELECT NOT 't' AS not_true, NOT 'f' AS not_false, NOT 'off' AS not_off;
