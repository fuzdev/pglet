-- The four settings whose value is a fact about the build rather than about the
-- box, so they are byte-comparable. `server_version` is deliberately absent:
-- 18.6 answers `18.6 (Debian 18.6-1.pgdg13+2)` and pglet answers `18.6`, which
-- `current_setting_server_version_num.sql` handles as a prefix test instead.
-- @test
SELECT
  current_setting('client_encoding') AS enc,
  current_setting('standard_conforming_strings') AS scs,
  current_setting('integer_datetimes') AS idt,
  current_setting('DateStyle') AS ds;
