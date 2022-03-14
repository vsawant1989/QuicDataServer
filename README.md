# QuicDataServer

To Run 


Run Server

cargo run --color=always --package QuicDataServer --bin server . --listen 127.0.0.1:8080 --stateless-retry --key resources/certs/key.der --cert resources/certs/cert.der


Run Client

cargo run --color=always --package QuicDataServer --bin client  http://127.0.0.1:8080/resources/transactions_4KB.json


Benchmarking:

cargo bench
