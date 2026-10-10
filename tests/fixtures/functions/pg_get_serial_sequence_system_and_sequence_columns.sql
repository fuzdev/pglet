-- A system column is a column (NULL, it owns nothing), and a sequence is a relation whose
-- columns are `last_value`, `log_cnt`, `is_called`. Strict over a NULL in either place.
-- @setup
CREATE TABLE ss (id serial);
-- @test
SELECT pg_get_serial_sequence('ss', 'xmin') AS xmin, pg_get_serial_sequence('ss', 'ctid') AS ctid,
  pg_get_serial_sequence('ss_id_seq', 'last_value') AS seq_column,
  pg_get_serial_sequence(NULL, 'id') AS null_table, pg_get_serial_sequence('ss', NULL) AS null_column;
