-- A header line is skipped without being split or de-escaped, so an escaped NUL on it is
-- never verified and the row after it lands, on both engines.
-- @setup
CREATE TABLE ct (a text);
-- @copy-from
COPY ct FROM STDIN (HEADER true);
-- @data
a\000
ok
-- @verify
SELECT a FROM ct;
