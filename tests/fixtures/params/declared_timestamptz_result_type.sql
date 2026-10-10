-- A declared `timestamptz` `$N`, projected bare, truncated and offset by an interval,
-- stays `timestamptz`; `date - date` over a declared `date` is `integer`.
-- @param-declared $1 timestamptz 2000-01-01T12:00:00+05:00
-- @param-declared $2 date 2000-01-11
-- @test
SELECT $1 AS bare, date_trunc('day', $1) AS truncated, $1 + interval '1 hour' AS later,
       $2 - date '2000-01-01' AS days;
