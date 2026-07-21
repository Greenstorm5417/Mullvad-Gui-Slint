#!/usr/bin/env node

import { readFileSync, writeFileSync } from "node:fs";

const source = JSON.parse(readFileSync("assets/geo/countries.geo.json", "utf8"));
const width = 640;
const height = 986;
const scale = 2;
const centerLatitude = (57.70887 * Math.PI) / 180;
const centerLongitude = (11.97456 * Math.PI) / 180;
const focalLength = height / 2 / Math.tan((70 * Math.PI) / 360);

function cameraOffset(distance) {
  return 0.088 + (distance - 1.25) * 0.3;
}

function project(coordinate, distance) {
  const longitude = (coordinate[0] * Math.PI) / 180;
  const latitude = (coordinate[1] * Math.PI) / 180;
  const deltaLongitude = longitude - centerLongitude;
  const cosLatitude = Math.cos(latitude);
  const x = cosLatitude * Math.sin(deltaLongitude);
  const y =
    Math.cos(centerLatitude) * Math.sin(latitude) -
    Math.sin(centerLatitude) * cosLatitude * Math.cos(deltaLongitude);
  const z =
    Math.sin(centerLatitude) * Math.sin(latitude) +
    Math.cos(centerLatitude) * cosLatitude * Math.cos(deltaLongitude);

  return {
    visible: z > 1 / distance,
    x: width / 2 + (focalLength * x) / (distance - z),
    y: height / 2 - (focalLength * (y + cameraOffset(distance))) / (distance - z),
  };
}

function ringsForGeometry(geometry) {
  if (geometry.type === "Polygon") return geometry.coordinates;
  if (geometry.type === "MultiPolygon") return geometry.coordinates.flat();
  return [];
}

function pathForRing(ring, distance) {
  const paths = [];
  let current = [];
  for (const coordinate of ring) {
    const point = project(coordinate, distance);
    if (point.visible) {
      current.push(`${point.x.toFixed(2)},${point.y.toFixed(2)}`);
    } else if (current.length > 1) {
      paths.push(`M${current.join("L")}Z`);
      current = [];
    } else {
      current = [];
    }
  }
  if (current.length > 1) paths.push(`M${current.join("L")}Z`);
  return paths.join("");
}

function generate(distance, destination) {
  const radius = focalLength / Math.sqrt(distance * distance - 1);
  const oceanCenterY =
    height / 2 -
    (focalLength * cameraOffset(distance) * distance) / (distance * distance - 1);
  const land = source.features
    .flatMap((feature) => ringsForGeometry(feature.geometry))
    .map((ring) => pathForRing(ring, distance))
    .filter(Boolean)
    .join("");

  const svg = `<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="${width / scale}" height="${height / scale}" viewBox="0 0 ${width} ${height}">
  <rect width="${width}" height="${height}" fill="#0a1923"/>
  <circle cx="${width / 2}" cy="${oceanCenterY.toFixed(2)}" r="${radius.toFixed(2)}" fill="#192e45"/>
  <path d="${land}" fill="#294d73" stroke="#192e45" stroke-width="1.5" stroke-linejoin="round"/>
</svg>`;
  writeFileSync(destination, svg);
}

generate(1.35, "assets/images/map-disconnected.svg");
generate(1.25, "assets/images/map-connected.svg");
