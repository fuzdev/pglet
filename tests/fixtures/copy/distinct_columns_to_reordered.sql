-- The accept polarity of the `duplicate_column_*` family: distinct names, in an order
-- that is not the table's, transfer exactly those columns in exactly that order. A gate
-- that refused a list merely for disagreeing with catalog order would redden this.
-- @setup
CREATE TABLE cda (a int, b int, c int);
INSERT INTO cda VALUES (1, 2, 3);
-- @copy-to
COPY cda (c, a) TO STDOUT;
