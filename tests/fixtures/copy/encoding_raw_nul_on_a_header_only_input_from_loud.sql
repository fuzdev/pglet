-- A raw NUL on a HEADER line, with no line after it. 18.6 answers `22021 invalid byte
-- sequence for encoding "UTF8": 0x00` and loads no row, although a header line is skipped
-- without being split or de-escaped: its raw-input check covers the header too. With no
-- data line after the header, a check placed behind pglet's header skip reaches no line
-- at all. A NUL is not written into a fixture file, so the data is spelled as bytes:
-- `h`, NUL, `d`, LF.
-- @setup
CREATE TABLE ct (a text);
-- @copy-from
COPY ct FROM STDIN (HEADER true);
-- @data-hex
68 00 64 0a
-- @verify
SELECT a FROM ct;
