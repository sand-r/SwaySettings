#!/usr/bin/env bash
set -euo pipefail

# Current AirPods IDs (do not reconnect while using this snapshot)
DEVICE_ID=166
SINK_ID=157
WRAPPER_ID=148

echo "### Debug snapshot @ $(date -Is)"
echo "DEVICE_ID=${DEVICE_ID} SINK_ID=${SINK_ID} WRAPPER_ID=${WRAPPER_ID}"
echo

echo "## wpctl status (Audio section)"
wpctl status | sed -n '/Audio/,/Video/p'
echo

echo "## wpctl inspect SINK (${SINK_ID})"
wpctl inspect "${SINK_ID}" | rg -n "node.name|media.class|api.bluez5.internal|bluez5.sink-loopback|bluez5.sink-loopback-target|node.driver-id|device.id|card.profile.device"
echo

echo "## wpctl inspect WRAPPER (${WRAPPER_ID})"
wpctl inspect "${WRAPPER_ID}" | rg -n "node.name|media.class|api.bluez5.internal|bluez5.sink-loopback|bluez5.sink-loopback-target|node.driver-id|device.id|card.profile.device"
echo

driver_id="$(wpctl inspect "${WRAPPER_ID}" 2>/dev/null | rg -o "node.driver-id = \"?([0-9]+)\"?" -r '$1' | head -n1 || true)"
if [[ -z "${driver_id}" ]]; then
  driver_id="$(wpctl inspect "${SINK_ID}" 2>/dev/null | rg -o "node.driver-id = \"?([0-9]+)\"?" -r '$1' | head -n1 || true)"
fi

if [[ -z "${driver_id}" ]]; then
  driver_id="${SINK_ID}"
  echo "## Driver id not found in inspect output; falling back to SINK_ID=${SINK_ID}"
else
  echo "## Detected driver id: ${driver_id}"
fi
echo

echo "## pw-cli info DEVICE (${DEVICE_ID}) params"
pw-cli info "${DEVICE_ID}" | rg -n "Param"
echo

echo "## pw-cli enum-params DEVICE (${DEVICE_ID}) Route (13)"
pw-cli enum-params "${DEVICE_ID}" 13 | rg -n "index|device|direction|available|name|description|channelVolumes|volume|mute"
echo

echo "## wpctl get-volume SINK (${SINK_ID})"
wpctl get-volume "${SINK_ID}" || true
echo

if [[ "${driver_id}" != "${SINK_ID}" ]]; then
  echo "## wpctl get-volume DRIVER (${driver_id})"
  wpctl get-volume "${driver_id}" || true
  echo
fi
