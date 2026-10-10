-- `client_encoding` is a `GUC_IS_NAME` setting, so 18.6 truncates its value to 63 bytes
-- (`truncate_identifier`, with a `NOTICE`) before looking the encoding up: `'utf8'` followed by
-- 60 spaces is `UTF8` there, as its first 63 bytes read.
-- @setup
SELECT 1;
SET client_encoding = 'utf8                                                            ';
-- @test
SELECT current_setting('client_encoding') AS client_encoding;
