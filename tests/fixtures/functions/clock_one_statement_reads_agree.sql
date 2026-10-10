-- Two reads of a transaction-start spelling inside one statement are one value, and so
-- are two reads of `statement_timestamp()`. 18.6 fixes the first family at the
-- transaction's start and the second at the statement's; only `clock_timestamp()` reads
-- the wall clock at the call, so it is compared with `<=` and never with itself.
--
-- The fixture runs inside the harness's block, so the statement starts after the
-- transaction did: `now() <= statement_timestamp()` holds and equality is not asserted.
-- @test
SELECT now() = now()                                   AS now_now,
       now() >= now()                                  AS now_ge_now,
       now() BETWEEN now() AND now()                   AS now_between,
       now() - now()                                   AS now_minus_now,
       now() = CURRENT_TIMESTAMP                       AS now_current_timestamp,
       now() = transaction_timestamp()                 AS now_transaction_timestamp,
       LOCALTIMESTAMP = LOCALTIMESTAMP                 AS localtimestamp_twice,
       now()::timestamp = LOCALTIMESTAMP               AS now_localtimestamp,
       now()::date = CURRENT_DATE                      AS now_current_date,
       CURRENT_TIME = CURRENT_TIME                     AS current_time_twice,
       LOCALTIME = LOCALTIME                           AS localtime_twice,
       statement_timestamp() = statement_timestamp()   AS statement_twice,
       now() <= statement_timestamp()                  AS transaction_before_statement,
       statement_timestamp() <= clock_timestamp()      AS statement_before_clock,
       now() <= clock_timestamp()                      AS transaction_before_clock;
