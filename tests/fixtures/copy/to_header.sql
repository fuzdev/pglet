-- COPY ... TO STDOUT WITH (HEADER) emits a leading column-name line, then the data
-- rows; the bytes (header + rows) must match Postgres.
-- @setup
CREATE TABLE h (id int, label text);
INSERT INTO h VALUES (1, 'one'), (2, 'two');
-- @copy-to
COPY h TO STDOUT WITH (HEADER);
