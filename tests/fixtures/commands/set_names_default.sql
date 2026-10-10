-- `SET NAMES DEFAULT` is `gram.y`'s `opt_encoding` default: it resets `client_encoding` as
-- `SET client_encoding TO DEFAULT` does, and answers `SET` on 18.6.
-- @setup
SELECT 1;
SET NAMES DEFAULT;
-- @test
SELECT current_setting('client_encoding') AS client_encoding;
