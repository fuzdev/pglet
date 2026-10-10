-- A raw NUL after the end-of-copy marker, on the marker's own line. 18.6 answers
-- `22021 invalid byte sequence for encoding "UTF8": 0x00` and loads no row, where a check
-- placed behind pglet's end-of-copy marker test answers that test's `22P04` instead. A NUL
-- is not written into a fixture file, so the data is spelled as bytes: `ok`, LF, a
-- backslash, a dot, NUL, LF.
-- @setup
CREATE TABLE ct (a text);
-- @copy-from
COPY ct FROM STDIN;
-- @data-hex
6f 6b 0a 5c 2e 00 0a
-- @verify
SELECT a FROM ct;
