import init, { PgletBtree } from '../../crates/pglet_wasm/pkg/pglet_wasm.js';

let db = null;

// SQL Examples for each button
const examples = {
    // === BASICS ===
    createTable: {
        desc: 'Create a products table with various column types',
        sql: `-- Create a table with common column types
CREATE TABLE products (
    id INT,
    name TEXT,
    price FLOAT,
    in_stock BOOL,
    category TEXT
);`
    },
    insertData: {
        desc: 'Insert sample product data',
        sql: `-- Insert multiple rows
INSERT INTO products VALUES
    (1, 'Laptop', 999.99, true, 'Electronics'),
    (2, 'Mouse', 29.99, true, 'Electronics'),
    (3, 'Keyboard', 79.99, true, 'Electronics'),
    (4, 'Desk Chair', 249.99, false, 'Furniture'),
    (5, 'Monitor Stand', 45.00, true, 'Furniture'),
    (6, 'USB Cable', 9.99, true, 'Electronics'),
    (7, 'Notebook', 4.99, true, 'Office'),
    (8, 'Pen Set', 12.99, false, 'Office');`
    },
    selectAll: {
        desc: 'Select all rows from products',
        sql: `-- Select all columns and rows
SELECT * FROM products ORDER BY id;`
    },
    update: {
        desc: 'Update product prices',
        sql: `-- Update: 10% off all electronics
UPDATE products SET price = price * 0.9 WHERE category = 'Electronics';

-- Verify the update
SELECT name, price, category FROM products WHERE category = 'Electronics' ORDER BY price;`
    },
    delete: {
        desc: 'Delete out-of-stock products',
        sql: `-- Delete products that are out of stock
DELETE FROM products WHERE in_stock = false;

-- Verify deletion
SELECT * FROM products ORDER BY id;`
    },
    resetDb: {
        desc: 'Drop all tables and start fresh',
        sql: `-- Drop all tables
DROP TABLE IF EXISTS products;
DROP TABLE IF EXISTS orders;
DROP TABLE IF EXISTS customers;
DROP TABLE IF EXISTS order_items;

-- Confirm tables are gone
SELECT 'Database reset complete!' AS status;`
    },

    // === QUERIES ===
    whereFilter: {
        desc: 'Filter with WHERE conditions',
        sql: `-- Multiple WHERE conditions with AND/OR
SELECT name, price, category
FROM products
WHERE (price > 20 AND category = 'Electronics')
   OR category = 'Furniture'
ORDER BY price DESC;`
    },
    orderBy: {
        desc: 'Sort results with ORDER BY',
        sql: `-- Sort by multiple columns
SELECT name, category, price
FROM products
ORDER BY category ASC, price DESC;`
    },
    limitOffset: {
        desc: 'Pagination with LIMIT and OFFSET',
        sql: `-- Get page 2 (items 3-4) of products sorted by price
SELECT name, price
FROM products
ORDER BY price DESC
LIMIT 2 OFFSET 2;`
    },
    like: {
        desc: 'Pattern matching with LIKE',
        sql: `-- Find products with names containing specific patterns
-- % matches any sequence, _ matches single character
SELECT name, category FROM products WHERE name LIKE '%a%';

SELECT name FROM products WHERE name LIKE 'M____';`
    },
    nullHandling: {
        desc: 'NULL values and IS NULL/IS NOT NULL',
        sql: `-- Create table with NULLs
DROP TABLE IF EXISTS employees;
CREATE TABLE employees (id INT, name TEXT, manager_id INT);
INSERT INTO employees VALUES
    (1, 'Alice', NULL),
    (2, 'Bob', 1),
    (3, 'Carol', 1),
    (4, 'Dave', NULL);

-- Find employees without a manager
SELECT name FROM employees WHERE manager_id IS NULL;

-- Find employees with a manager
SELECT name, manager_id FROM employees WHERE manager_id IS NOT NULL;`
    },
    cast: {
        desc: 'Type casting with CAST and ::',
        sql: `-- Cast between types
SELECT
    CAST(123 AS TEXT) AS int_to_text,
    CAST('456' AS INT) AS text_to_int,
    42::TEXT AS shorthand_cast,
    3.14159::INT AS float_to_int;`
    },

    // === AGGREGATES ===
    countSumAvg: {
        desc: 'COUNT, SUM, and AVG aggregates',
        sql: `-- Aggregate functions
SELECT
    COUNT(*) AS total_products,
    COUNT(DISTINCT category) AS num_categories,
    SUM(price) AS total_value,
    AVG(price) AS avg_price
FROM products;`
    },
    minMax: {
        desc: 'MIN and MAX aggregates',
        sql: `-- Find price extremes
SELECT
    MIN(price) AS cheapest,
    MAX(price) AS most_expensive,
    MAX(price) - MIN(price) AS price_range
FROM products;`
    },
    groupBy: {
        desc: 'GROUP BY for category summaries',
        sql: `-- Summary by category
SELECT
    category,
    COUNT(*) AS num_products,
    SUM(price) AS total_value,
    AVG(price) AS avg_price
FROM products
GROUP BY category
ORDER BY total_value DESC;`
    },
    having: {
        desc: 'Filter groups with HAVING',
        sql: `-- Categories with more than 2 products
SELECT
    category,
    COUNT(*) AS num_products,
    AVG(price) AS avg_price
FROM products
GROUP BY category
HAVING COUNT(*) > 2
ORDER BY avg_price DESC;`
    },
    distinct: {
        desc: 'Remove duplicates with DISTINCT',
        sql: `-- Get unique categories
SELECT DISTINCT category FROM products ORDER BY category;

-- Count distinct values
SELECT COUNT(DISTINCT category) AS unique_categories FROM products;`
    },

    // === JOINS ===
    setupJoins: {
        desc: 'Create customers and orders tables for JOIN examples',
        sql: `-- Setup tables for JOIN examples
DROP TABLE IF EXISTS order_items;
DROP TABLE IF EXISTS orders;
DROP TABLE IF EXISTS customers;

CREATE TABLE customers (
    id INT,
    name TEXT,
    email TEXT
);

CREATE TABLE orders (
    id INT,
    customer_id INT,
    total FLOAT,
    status TEXT
);

INSERT INTO customers VALUES
    (1, 'Alice', 'alice@example.com'),
    (2, 'Bob', 'bob@example.com'),
    (3, 'Carol', 'carol@example.com'),
    (4, 'Dave', 'dave@example.com');

INSERT INTO orders VALUES
    (101, 1, 150.00, 'shipped'),
    (102, 1, 75.50, 'delivered'),
    (103, 2, 200.00, 'pending'),
    (104, 5, 50.00, 'shipped');

SELECT 'Tables created!' AS status;
SELECT * FROM customers;
SELECT * FROM orders;`
    },
    innerJoin: {
        desc: 'INNER JOIN - only matching rows',
        sql: `-- INNER JOIN: customers with orders
SELECT
    c.name AS customer,
    o.id AS order_id,
    o.total,
    o.status
FROM customers c
INNER JOIN orders o ON c.id = o.customer_id
ORDER BY c.name;`
    },
    leftJoin: {
        desc: 'LEFT JOIN - all left rows, matching right',
        sql: `-- LEFT JOIN: all customers, with orders if any
SELECT
    c.name AS customer,
    o.id AS order_id,
    o.total
FROM customers c
LEFT JOIN orders o ON c.id = o.customer_id
ORDER BY c.name;`
    },
    rightJoin: {
        desc: 'RIGHT JOIN - all right rows, matching left',
        sql: `-- RIGHT JOIN: all orders, with customer if exists
SELECT
    c.name AS customer,
    o.id AS order_id,
    o.total,
    o.status
FROM customers c
RIGHT JOIN orders o ON c.id = o.customer_id
ORDER BY o.id;`
    },
    fullJoin: {
        desc: 'FULL JOIN - all rows from both tables',
        sql: `-- FULL JOIN: all customers and all orders
SELECT
    c.name AS customer,
    o.id AS order_id,
    o.total
FROM customers c
FULL JOIN orders o ON c.id = o.customer_id
ORDER BY c.name, o.id;`
    },

    // === ADVANCED ===
    subquery: {
        desc: 'Subquery with IN clause',
        sql: `-- Find customers who have placed orders
SELECT name, email
FROM customers
WHERE id IN (SELECT DISTINCT customer_id FROM orders)
ORDER BY name;

-- Find products more expensive than average
SELECT name, price
FROM products
WHERE price > (SELECT AVG(price) FROM products)
ORDER BY price DESC;`
    },
    scalarSubquery: {
        desc: 'Scalar subquery in SELECT',
        sql: `-- Include aggregate in each row
SELECT
    name,
    price,
    (SELECT AVG(price) FROM products) AS avg_price,
    price - (SELECT AVG(price) FROM products) AS diff_from_avg
FROM products
ORDER BY diff_from_avg DESC;`
    },
    derivedTable: {
        desc: 'Subquery in FROM clause (derived table)',
        sql: `-- Use subquery result as a table
SELECT category, avg_price
FROM (
    SELECT category, AVG(price) AS avg_price
    FROM products
    GROUP BY category
) AS category_stats
WHERE avg_price > 20
ORDER BY avg_price DESC;`
    },
    cte: {
        desc: 'Common Table Expression (WITH clause)',
        sql: `-- CTE for readable complex queries
WITH expensive_products AS (
    SELECT * FROM products WHERE price > 50
),
category_counts AS (
    SELECT category, COUNT(*) AS cnt
    FROM expensive_products
    GROUP BY category
)
SELECT * FROM category_counts ORDER BY cnt DESC;`
    },
    windowFn: {
        desc: 'Window functions: ROW_NUMBER, RANK',
        sql: `-- Rank products by price within category
SELECT
    name,
    category,
    price,
    ROW_NUMBER() OVER (PARTITION BY category ORDER BY price DESC) AS rank_in_category,
    RANK() OVER (ORDER BY price DESC) AS overall_rank
FROM products
ORDER BY category, price DESC;`
    },
    upsert: {
        desc: 'INSERT ... ON CONFLICT (upsert)',
        sql: `-- Create table with unique column
DROP TABLE IF EXISTS settings;
CREATE TABLE settings (key TEXT, value TEXT);
INSERT INTO settings VALUES ('theme', 'light'), ('lang', 'en');

-- Upsert: insert or update on conflict
INSERT INTO settings VALUES ('theme', 'dark')
ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value;

INSERT INTO settings VALUES ('font_size', '14')
ON CONFLICT (key) DO NOTHING;

SELECT * FROM settings ORDER BY key;`
    },
    returning: {
        desc: 'RETURNING clause for INSERT/UPDATE/DELETE',
        sql: `-- INSERT with RETURNING
INSERT INTO products VALUES (100, 'New Item', 19.99, true, 'Test')
RETURNING *;

-- UPDATE with RETURNING
UPDATE products SET price = 24.99 WHERE id = 100
RETURNING id, name, price AS new_price;

-- DELETE with RETURNING
DELETE FROM products WHERE id = 100
RETURNING name, 'deleted' AS action;`
    },
    transaction: {
        desc: 'Transaction with BEGIN/COMMIT/ROLLBACK',
        sql: `-- Show initial state
SELECT name, price FROM products WHERE id = 1;

-- Start transaction
BEGIN;
UPDATE products SET price = 0 WHERE id = 1;
SELECT name, price FROM products WHERE id = 1;

-- Rollback the change
ROLLBACK;

-- Price is restored
SELECT name, price FROM products WHERE id = 1;`
    }
};

async function initDB() {
    const badge = document.getElementById('statusBadge');
    try {
        await init();
        db = new PgletBtree();
        badge.textContent = 'Ready (In-Memory)';
        badge.className = 'status-badge status-ready';
        console.log('pglet initialized');
    } catch (error) {
        badge.textContent = 'Error';
        badge.className = 'status-badge status-memory';
        throw error;
    }
}

function formatResult(result) {
    if (!result || !result.rows || result.rows.length === 0) {
        const rowCount = result?.rowCount || 0;
        return `<div class="success">OK${rowCount > 0 ? ` (${rowCount} row${rowCount !== 1 ? 's' : ''} affected)` : ''}</div>`;
    }

    const fields = result.fields || [];
    const rows = result.rows || [];

    let output = '<table><tr>';
    for (const field of fields) {
        output += `<th>${field.name}</th>`;
    }
    output += '</tr>';

    for (const row of rows) {
        output += '<tr>';
        for (const field of fields) {
            const value = row[field.name];
            const display = value === null || value === undefined ? '<span style="color:#666">NULL</span>' : value;
            output += `<td>${display}</td>`;
        }
        output += '</tr>';
    }
    output += '</table>';
    output += `<div class="info">${rows.length} row${rows.length !== 1 ? 's' : ''}</div>`;

    return output;
}

function executeStatement(stmt) {
    try {
        const result = db.query(stmt);
        return { success: true, result };
    } catch (e) {
        try {
            db.exec(stmt);
            return { success: true, result: null };
        } catch (e2) {
            return { success: false, error: String(e2) };
        }
    }
}

window.runSQL = async function() {
    if (!db) await initDB();

    const sqlInput = document.getElementById('sqlInput');
    const output = document.getElementById('output');
    const sql = sqlInput.value.trim();

    if (!sql) {
        output.innerHTML = '<span class="error">Please enter SQL to execute</span>';
        return;
    }

    output.innerHTML = '<span class="info">Executing...</span>';

    try {
        // Split by semicolon, strip comments from each statement, filter empty
        const statements = sql.split(';')
            .map(s => s.trim())
            .map(s => stripLeadingComments(s))
            .filter(s => s.length > 0);
        let html = '';

        for (const stmt of statements) {
            const { success, result, error } = executeStatement(stmt);
            html += '<div style="margin-bottom: 15px;">';
            html += `<div class="query-label">${escapeHtml(truncate(stmt, 80))}</div>`;
            if (success) {
                html += formatResult(result);
            } else {
                html += `<div class="error">Error: ${escapeHtml(error)}</div>`;
            }
            html += '</div>';
        }

        output.innerHTML = html;
    } catch (error) {
        output.innerHTML = `<span class="error">Error: ${escapeHtml(String(error))}</span>`;
    }
};

window.runExample = async function(name) {
    if (!db) await initDB();

    const example = examples[name];
    if (!example) {
        console.error('Unknown example:', name);
        return;
    }

    const sqlInput = document.getElementById('sqlInput');
    sqlInput.value = example.sql;

    // Auto-run
    await window.runSQL();
};

window.clearOutput = function() {
    document.getElementById('output').innerHTML = '<span class="info">Output cleared.</span>';
};

window.showSchema = async function() {
    if (!db) await initDB();

    const schemaInfo = document.getElementById('schemaInfo');

    try {
        const result = db.query(`
            SELECT relname AS table_name
            FROM pg_class
            WHERE relkind = 'r'
            ORDER BY relname
        `);

        if (!result.rows || result.rows.length === 0) {
            schemaInfo.textContent = 'No tables found. Click "Create Table" to get started.';
        } else {
            const tables = result.rows.map(r => r.table_name);
            let info = `Tables: ${tables.join(', ')}\n\n`;

            // Get columns for each table using simple query
            for (const table of tables) {
                try {
                    const cols = db.query(`
                        SELECT attname AS column_name
                        FROM pg_attribute
                        WHERE attrelid = (SELECT oid FROM pg_class WHERE relname = '${table}')
                        AND attnum > 0
                        ORDER BY attnum
                    `);
                    if (cols.rows && cols.rows.length > 0) {
                        const colList = cols.rows.map(c => c.column_name).join(', ');
                        info += `${table}: ${colList}\n`;
                    }
                } catch (e) {
                    info += `${table}: (columns unavailable)\n`;
                }
            }
            schemaInfo.textContent = info;
        }
        schemaInfo.style.display = 'block';
    } catch (error) {
        schemaInfo.textContent = 'Error fetching schema: ' + error;
        schemaInfo.style.display = 'block';
    }
};

function escapeHtml(str) {
    return str.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

function truncate(str, len) {
    const oneLine = str.replace(/\s+/g, ' ');
    return oneLine.length > len ? oneLine.slice(0, len) + '...' : oneLine;
}

// Strip leading comment lines from a SQL statement
function stripLeadingComments(sql) {
    const lines = sql.split('\n');
    const nonCommentLines = [];
    let foundNonComment = false;

    for (const line of lines) {
        const trimmed = line.trim();
        if (foundNonComment || (trimmed && !trimmed.startsWith('--'))) {
            foundNonComment = true;
            nonCommentLines.push(line);
        }
    }

    return nonCommentLines.join('\n').trim();
}

// Initialize on load
initDB().catch(error => {
    console.error('Failed to initialize pglet:', error);
    document.getElementById('output').innerHTML =
        `<span class="error">Failed to initialize: ${error.message || error}</span>`;
});
