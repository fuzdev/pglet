-- `timestamp::time` / `timestamptz::time` truncate to the time-of-day µs. pglet is
-- UTC-only, so both casts agree; the harness pins TimeZone=UTC so Postgres matches.
-- @test
SELECT '2024-03-04 15:30:45.5'::timestamp::time    AS from_ts,
       '2024-03-04 15:30:45+00'::timestamptz::time AS from_tstz;
