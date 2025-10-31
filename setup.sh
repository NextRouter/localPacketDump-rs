#!/bin/bash

# Check if build was successful
if [ $? -ne 0 ]; then
    echo "Build failed. Exiting."
    exit 1
fi

/home/user/.cargo/bin/cargo build --release

# Create systemd service file
echo "Creating systemd service..."
SERVICE_FILE="/etc/systemd/system/localpacketdump.service"
CURRENT_DIR=$(pwd)
BINARY_PATH="$CURRENT_DIR/target/release/localpacketdump"

# Check if binary exists
if [ ! -f "$BINARY_PATH" ]; then
    echo "Error: Binary not found at $BINARY_PATH"
    exit 1
fi

tee $SERVICE_FILE > /dev/null << EOF
[Unit]
Description=Local Packet Dump Service
After=network.target

[Service]
Type=simple
ExecStart=$BINARY_PATH
WorkingDirectory=$CURRENT_DIR
Restart=always
RestartSec=5
User=root

[Install]
WantedBy=multi-user.target
EOF

# Reload systemd and enable the service
sudo systemctl daemon-reload
sudo systemctl enable localpacketdump.service

echo "Service created and enabled. You can start it with:"
echo "sudo systemctl start localpacketdump.service"