-- `LOCALTIMESTAMP` — the SQL value function (parenless) for the current timestamp
-- **without** time zone (`timestamp`), the same instant as `CURRENT_TIMESTAMP`
-- (`timestamptz`) but typed/rendered without the `+00` offset. Its raw value is a
-- moving clock, so compare only deterministic properties: all three value functions
-- below share one statement timestamp, so `::date` equals `CURRENT_DATE` and the
-- instant is `<= CURRENT_TIMESTAMP`. All resolve to a deterministic `true`.
-- @test
SELECT LOCALTIMESTAMP IS NOT NULL              AS not_null,
       LOCALTIMESTAMP::date = CURRENT_DATE     AS is_today,
       LOCALTIMESTAMP <= CURRENT_TIMESTAMP     AS ordered;
