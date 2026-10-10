-- A NUL written as a COPY escape (`\000`) is 18.6's `22021 invalid byte sequence for
-- encoding "UTF8": 0x00` — `CopyReadAttributesText` verifies the de-escaped field in the
-- server encoding — and no row lands. The harness compares SQLSTATEs only; the in-process
-- `copy` tests pin the byte list.
-- @setup
CREATE TABLE ct (a text);
-- @copy-from
COPY ct FROM STDIN;
-- @data
x\000y
-- @verify
SELECT a FROM ct;
