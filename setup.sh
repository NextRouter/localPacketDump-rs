#!/bin/bash

# Check for test mode
if [ "$1" = "test" ]; then
    echo "Running in test mode..."
    /home/user/.cargo/bin/cargo build --release
    if [ $? -ne 0 ]; then
        echo "Build failed. Exiting."
        exit 1
    fi
    
    echo "Starting localpacketdump in test mode..."
    echo "Note: This requires root privileges to capture packets."
    echo "Press Ctrl+C to stop."
    sudo ./target/release/localpacketdump
    exit 0
fi

# Normal setup mode
echo "Building localpacketdump..."

/home/user/.cargo/bin/cargo build --release

# Check if build was successful
if [ $? -ne 0 ]; then
    echo "Build failed. Exiting."
    exit 1
fi

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

# Check if running on Linux with systemd
if command -v systemctl &> /dev/null; then
    # Reload systemd and enable the service
    sudo systemctl daemon-reload
    sudo systemctl enable localpacketdump.service

    echo "Service created and enabled. You can start it with:"
    echo "sudo systemctl start localpacketdump.service"
else
    echo "systemd not found. Service file created but not installed."
    echo "To run manually, execute:"
    echo "sudo ./target/release/localpacketdump"
fi